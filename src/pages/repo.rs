use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

use crate::components::{HeadingEntry, SpecOutline, SpecSidebar};

#[cfg(feature = "ssr")]
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoInfo {
    pub id: i32,
    pub github_url: String,
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderedFile {
    pub path: String,
    pub html: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderedSpec {
    pub id: i32,
    pub name: String,
    pub files: Vec<RenderedFile>,
    pub headings: Vec<HeadingEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalInfo {
    pub id: i32,
    pub title: Option<String>,
    pub status: String,
}

#[server]
pub async fn get_repository(repo_id: i32) -> Result<RepoInfo, ServerFnError> {
    use diesel::prelude::*;

    info!(repo_id, "get_repository called");

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::repositories::dsl::*;
        let repo = repositories
            .filter(id.eq(repo_id))
            .select((id, github_url, owner, name))
            .first::<(i32, String, String, String)>(conn)?;
        Ok::<_, diesel::result::Error>(RepoInfo {
            id: repo.0,
            github_url: repo.1,
            owner: repo.2,
            name: repo.3,
        })
    })
    .await
    .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
    .map_err(|e| {
        tracing::error!(repo_id, error = %e, "get_repository query failed");
        ServerFnError::new(format!("query error: {e}"))
    })
}

#[cfg(feature = "ssr")]
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c == ' ' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .collect()
}

#[cfg(feature = "ssr")]
fn extract_headings(content: &str) -> Vec<HeadingEntry> {
    // path holds (level, slug) for each ancestor heading currently in scope.
    let mut path: Vec<(u8, String)> = Vec::new();
    let mut entries = Vec::new();

    for line in content.lines() {
        let n = line.chars().take_while(|&c| c == '#').count();
        if n == 0 || n > 6 {
            continue;
        }
        let rest = &line[n..];
        if !rest.starts_with(' ') {
            continue;
        }
        let text = rest.trim().to_string();
        if text.is_empty() {
            continue;
        }

        let slug = slugify(&text);

        // Pop any entries at the same level or deeper so the path only
        // contains true ancestors of the current heading.
        while path.last().map(|(l, _)| *l >= n as u8).unwrap_or(false) {
            path.pop();
        }
        path.push((n as u8, slug));

        // marq anchors are the full ancestor path joined with "--".
        let anchor = path
            .iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join("--");

        entries.push(HeadingEntry {
            level: n as u8,
            text,
            anchor,
        });
    }

    entries
}

#[server]
pub async fn list_rendered_specs(repo_id: i32) -> Result<Vec<RenderedSpec>, ServerFnError> {
    use diesel::prelude::*;
    use marq::{RenderOptions, render};

    info!(repo_id, "list_rendered_specs called");

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    // r[impl repo.multi-spec]
    // r[impl repo.multi-file]
    let raw: Vec<(i32, String, String, String)> = conn
        .interact(move |conn| {
            use crate::db::schema::{spec_files, specs};
            specs::table
                .filter(specs::repository_id.eq(repo_id))
                .inner_join(spec_files::table)
                .order((specs::name.asc(), spec_files::path.asc()))
                .select((
                    specs::id,
                    specs::name,
                    spec_files::path,
                    spec_files::content,
                ))
                .load(conn)
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e: diesel::result::Error| {
            tracing::error!(repo_id, error = %e, "list_rendered_specs query failed");
            ServerFnError::new(format!("query error: {e}"))
        })?;

    // Group consecutive rows by spec (query is ordered by spec name).
    type GroupedSpec = (i32, String, Vec<(String, String)>);
    let mut grouped: Vec<GroupedSpec> = Vec::new();
    for (spec_id, spec_name, file_path, file_content) in raw {
        if let Some(last) = grouped.last_mut()
            && last.0 == spec_id
        {
            last.2.push((file_path, file_content));
            continue;
        }
        grouped.push((spec_id, spec_name, vec![(file_path, file_content)]));
    }

    let mut result = Vec::with_capacity(grouped.len());
    for (spec_id, spec_name, files) in grouped {
        let headings: Vec<HeadingEntry> = files
            .iter()
            .flat_map(|(_, content)| extract_headings(content))
            .collect();

        let mut rendered_files = Vec::with_capacity(files.len());
        for (path, content) in files {
            let opts = RenderOptions::new().with_source_path(&path);
            // r[impl view.render]
            let doc = render(&content, &opts).await.map_err(|e| {
                tracing::error!(repo_id, %path, error = %e, "marq render failed");
                ServerFnError::new(format!("Failed to render {path}: {e}"))
            })?;
            rendered_files.push(RenderedFile {
                path,
                html: doc.html,
            });
        }
        result.push(RenderedSpec {
            id: spec_id,
            name: spec_name,
            files: rendered_files,
            headings,
        });
    }

    info!(
        repo_id,
        spec_count = result.len(),
        "list_rendered_specs returned"
    );
    Ok(result)
}

#[server]
pub async fn list_proposals(repo_id: i32) -> Result<Vec<ProposalInfo>, ServerFnError> {
    use diesel::prelude::*;

    info!(repo_id, "list_proposals called");

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    let result = conn
        .interact(move |conn| {
            use crate::db::schema::proposals::dsl::*;
            // r[impl proposal.multiple.overview]
            let rows = proposals
                .filter(repository_id.eq(repo_id))
                .order(created_at.desc())
                .select((id, title, status))
                .load::<(i32, Option<String>, String)>(conn)?;
            Ok::<_, diesel::result::Error>(
                rows.into_iter()
                    .map(|(pid, ptitle, pstatus)| ProposalInfo {
                        id: pid,
                        title: ptitle,
                        status: pstatus,
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e| {
            tracing::error!(repo_id, error = %e, "list_proposals query failed");
            ServerFnError::new(format!("query error: {e}"))
        })?;

    info!(repo_id, count = result.len(), "list_proposals returned");
    Ok(result)
}

#[component]
pub fn RepoPage() -> impl IntoView {
    let params = use_params_map();
    let repo_id = move || {
        params
            .read()
            .get("repo_id")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    };

    let repo = Resource::new(repo_id, get_repository);
    let specs_resource = Resource::new(repo_id, list_rendered_specs);
    let proposals_resource = Resource::new(repo_id, list_proposals);

    view! {
        // ── Header ──────────────────────────────────────────────────────────
        <Suspense fallback=move || view! { <p>"Loading..."</p> }>
            {move || {
                repo.get().map(|result| match result {
                    Ok(r) => {
                        let label = format!("{}/{}", r.owner, r.name);
                        let url_href = r.github_url.clone();
                        let url_text = r.github_url;
                        view! {
                            <div class="level mb-2">
                                <div class="level-left">
                                    <div class="level-item">
                                        <div>
                                            <h1 class="title mb-1">{label}</h1>
                                            <p class="subtitle is-6">
                                                <a href=url_href target="_blank">{url_text}</a>
                                            </p>
                                        </div>
                                    </div>
                                </div>
                                <div class="level-right">
                                    <div class="level-item">
                                        // r[impl users.collaboration]
                                        <a
                                            class="button is-primary"
                                            href=move || format!("/repo/{}/proposal/new", repo_id())
                                        >
                                            "Propose a Change"
                                        </a>
                                    </div>
                                </div>
                            </div>
                        }.into_any()
                    }
                    Err(e) => view! {
                        <div class="notification is-danger">{format!("Error: {e}")}</div>
                    }.into_any(),
                })
            }}
        </Suspense>

        // ── Proposals bar ────────────────────────────────────────────────────
        <Suspense fallback=|| ()>
            {move || {
                proposals_resource.get().map(|result| match result {
                    Ok(list) if !list.is_empty() => {
                        let rid = repo_id();
                        view! {
                            <div class="box py-3 mb-5">
                                <p class="has-text-grey is-size-7 mb-2">"Open proposals"</p>
                                <div class="tags">
                                    {list.into_iter().map(|p| {
                                        let label = p.title
                                            .unwrap_or_else(|| format!("Proposal #{}", p.id));
                                        let tag_class = match p.status.as_str() {
                                            "drafting"    => "tag is-warning",
                                            "in_progress" => "tag is-info",
                                            "merged"      => "tag is-success",
                                            "abandoned"   => "tag is-danger",
                                            _             => "tag is-light",
                                        };
                                        view! {
                                            <a
                                                class=tag_class
                                                href=format!("/repo/{}/proposal/{}", rid, p.id)
                                            >
                                                {label}
                                            </a>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </div>
                        }.into_any()
                    }
                    _ => ().into_any(),
                })
            }}
        </Suspense>

        // ── Spec content ─────────────────────────────────────────────────────
        <Suspense fallback=move || view! { <p>"Loading specs..."</p> }>
            {move || {
                specs_resource.get().map(|result| match result {
                    Ok(specs) => {
                        if specs.is_empty() {
                            view! {
                                <p class="has-text-grey">
                                    "No specs found for this repository."
                                </p>
                            }.into_any()
                        } else {
                            let outline: Vec<SpecOutline> = specs
                                .iter()
                                .map(|s| SpecOutline {
                                    name: s.name.clone(),
                                    headings: s.headings.clone(),
                                })
                                .collect();

                            view! {
                                // r[impl repo.multi-spec]
                                <div style="display: flex; align-items: flex-start; margin: 0 -1.5rem;">
                                    <SpecSidebar specs=outline />
                                    <div style="flex: 1; min-width: 0; padding: 0 1.5rem;">
                                        {specs.into_iter().map(|spec| {
                                            view! {
                                                <section class="mb-6">
                                                    {spec.files.into_iter().map(|file| {
                                                        view! {
                                                            <div class="content spec-content"
                                                                inner_html=file.html />
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </section>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            }.into_any()
                        }
                    }
                    Err(e) => view! {
                        <div class="notification is-danger">
                            {format!("Error loading specs: {e}")}
                        </div>
                    }.into_any(),
                })
            }}
        </Suspense>
    }
}
