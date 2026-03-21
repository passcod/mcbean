use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;

use crate::components::ProposalFab;
use serde::{Deserialize, Serialize};

use crate::components::{HeadingEntry, SearchEntry, SpecOutline, SpecSidebar};

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
    pub search_entries: Vec<SearchEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalInfo {
    pub id: i32,
    pub title: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserOpenProposal {
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
        let mut rendered_files = Vec::with_capacity(files.len());
        let mut headings: Vec<HeadingEntry> = Vec::new();
        let mut search_entries: Vec<SearchEntry> = Vec::new();

        for (path, content) in files {
            let opts = RenderOptions::new().with_source_path(&path);
            // r[impl view.render]
            let doc = render(&content, &opts).await.map_err(|e| {
                tracing::error!(repo_id, %path, error = %e, "marq render failed");
                ServerFnError::new(format!("Failed to render {path}: {e}"))
            })?;

            for h in &doc.headings {
                headings.push(HeadingEntry {
                    level: h.level,
                    text: h.title.clone(),
                    anchor: h.id.clone(),
                });
            }

            // r[impl view.search]
            // Walk elements in document order, tracking the nearest heading
            // anchor so paragraph/req entries can link back to their section.
            let mut current_anchor = String::new();
            for element in &doc.elements {
                use marq::DocElement;
                match element {
                    DocElement::Heading(h) => {
                        current_anchor = h.id.clone();
                        search_entries.push(SearchEntry {
                            spec_name: spec_name.clone(),
                            text: h.title.clone(),
                            anchor: h.id.clone(),
                        });
                    }
                    DocElement::Req(r) => {
                        let text = r
                            .raw
                            .lines()
                            .map(|line| {
                                line.strip_prefix("> ")
                                    .or_else(|| line.strip_prefix('>'))
                                    .unwrap_or(line)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                            .trim()
                            .to_string();
                        search_entries.push(SearchEntry {
                            spec_name: spec_name.clone(),
                            text,
                            anchor: r.anchor_id.clone(),
                        });
                    }
                    DocElement::Paragraph(p) => {
                        let start = p.offset;
                        if start < content.len() {
                            let rest = &content[start..];
                            let end = rest.find("\n\n").unwrap_or(rest.len());
                            let text = rest[..end].trim().to_string();
                            if !text.is_empty() {
                                search_entries.push(SearchEntry {
                                    spec_name: spec_name.clone(),
                                    text,
                                    anchor: current_anchor.clone(),
                                });
                            }
                        }
                    }
                }
            }

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
            search_entries,
        });
    }

    info!(
        repo_id,
        spec_count = result.len(),
        "list_rendered_specs returned"
    );
    Ok(result)
}

/// Returns the latest open proposal (drafting or in_progress) for this repo
/// that the current user created or has contributed a change to.
// r[impl proposal.multiple.warning]
#[server]
pub async fn get_user_open_proposal(
    repo_id: i32,
) -> Result<Option<UserOpenProposal>, ServerFnError> {
    use diesel::prelude::*;

    let user_id = crate::auth::get_or_create_user_id().await?;
    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let result = conn
        .interact(move |conn| {
            use crate::db::schema::{proposal_changes, proposals};

            // All open proposals for this repo where the user is the creator
            // or has authored at least one change.
            let row = proposals::table
                .left_join(
                    proposal_changes::table.on(proposal_changes::proposal_id
                        .eq(proposals::id)
                        .and(proposal_changes::user_id.eq(user_id))),
                )
                .filter(proposals::repository_id.eq(repo_id))
                .filter(
                    proposals::status
                        .eq("drafting")
                        .or(proposals::status.eq("in_progress")),
                )
                .filter(
                    proposals::created_by
                        .eq(user_id)
                        .or(proposal_changes::user_id.eq(user_id)),
                )
                .order(proposals::id.desc())
                .select((proposals::id, proposals::title, proposals::status))
                .distinct()
                .first::<(i32, Option<String>, String)>(conn)
                .optional()?;

            Ok::<_, diesel::result::Error>(row.map(|(id, title, status)| UserOpenProposal {
                id,
                title,
                status,
            }))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

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
                            <Title text=label.clone()/>
                            <div class="level mb-2">
                                <div class="level-left">
                                    <div class="level-item">
                                        <div>
                                            <h1 class="title mb-1">{label.clone()}</h1>
                                            <p class="subtitle is-6">
                                                <a href=url_href target="_blank">{url_text}</a>
                                            </p>
                                        </div>
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
                let rid = repo_id();
                proposals_resource.get().map(|result| match result {
                    Ok(list) if !list.is_empty() => {
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
                            let all_search_entries: Vec<SearchEntry> = specs
                                .iter()
                                .flat_map(|s| s.search_entries.iter().cloned())
                                .collect();

                            view! {
                                // r[impl repo.multi-spec]
                                <div style="display: flex; align-items: flex-start; margin: 0 -1.5rem;">
                                    <SpecSidebar outline=outline search_entries=all_search_entries />
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

        // r[impl users.collaboration]
        <ProposalFab repo_id=repo_id() />
    }
}
