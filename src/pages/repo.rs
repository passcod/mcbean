use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoInfo {
    pub id: i32,
    pub github_url: String,
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecInfo {
    pub id: i32,
    pub name: String,
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
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e| ServerFnError::new(format!("{e}")))
}

#[server]
pub async fn list_specs(repo_id: i32) -> Result<Vec<SpecInfo>, ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::specs::dsl::*;
        // r[impl repo.multi-spec]
        let results = specs
            .filter(repository_id.eq(repo_id))
            .select((id, name))
            .load::<(i32, String)>(conn)?;
        Ok::<_, diesel::result::Error>(
            results
                .into_iter()
                .map(|(sid, sname)| SpecInfo {
                    id: sid,
                    name: sname,
                })
                .collect(),
        )
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e| ServerFnError::new(format!("{e}")))
}

#[server]
pub async fn list_proposals(repo_id: i32) -> Result<Vec<ProposalInfo>, ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::proposals::dsl::*;
        // r[impl proposal.multiple.overview]
        let results = proposals
            .filter(repository_id.eq(repo_id))
            .select((id, title, status))
            .load::<(i32, Option<String>, String)>(conn)?;
        Ok::<_, diesel::result::Error>(
            results
                .into_iter()
                .map(|(pid, ptitle, pstatus)| ProposalInfo {
                    id: pid,
                    title: ptitle,
                    status: pstatus,
                })
                .collect(),
        )
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e| ServerFnError::new(format!("{e}")))
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
    let specs_resource = Resource::new(repo_id, list_specs);
    let proposals_resource = Resource::new(repo_id, list_proposals);

    let (active_tab, set_active_tab) = signal("specs".to_string());

    view! {
        <Suspense fallback=move || {
            view! { <p>"Loading repository..."</p> }
        }>
            {move || {
                repo.get()
                    .map(|result| match result {
                        Ok(r) => {
                            let label = format!("{}/{}", r.owner, r.name);
                            let url_href = r.github_url.clone();
                            let url_text = r.github_url;
                            view! {
                                <h1 class="title">{label}</h1>
                                <p class="subtitle">
                                    <a href=url_href target="_blank">
                                        {url_text}
                                    </a>
                                </p>
                            }
                                .into_any()
                        }
                        Err(e) => {
                            view! {
                                <div class="notification is-danger">{format!("Error: {e}")}</div>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>

        <div class="tabs">
            <ul>
                <li class:is-active=move || active_tab.get() == "specs">
                    <a on:click=move |_| set_active_tab.set("specs".to_string())>"Specs"</a>
                </li>
                <li class:is-active=move || active_tab.get() == "proposals">
                    <a on:click=move |_| set_active_tab.set("proposals".to_string())>
                        "Proposals"
                    </a>
                </li>
            </ul>
        </div>

        <div style:display=move || {
            if active_tab.get() == "specs" { "block" } else { "none" }
        }>
            <Suspense fallback=move || {
                view! { <p>"Loading specs..."</p> }
            }>
                {move || {
                    specs_resource
                        .get()
                        .map(|result| match result {
                            Ok(spec_list) => {
                                if spec_list.is_empty() {
                                    view! {
                                        <p class="has-text-grey">"No specs found for this repository."</p>
                                    }
                                        .into_any()
                                } else {
                                    let rid = repo_id();
                                    view! {
                                        // r[impl repo.multi-spec]
                                        <div class="list">
                                            {spec_list
                                                .into_iter()
                                                .map(|s| {
                                                    view! {
                                                        <a
                                                            class="list-item"
                                                            href=format!("/repo/{}/spec/{}", rid, s.id)
                                                        >
                                                            {s.name}
                                                        </a>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </div>
                                    }
                                        .into_any()
                                }
                            }
                            Err(e) => {
                                view! {
                                    <div class="notification is-danger">
                                        {format!("Error: {e}")}
                                    </div>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        </div>

        <div style:display=move || {
            if active_tab.get() == "proposals" { "block" } else { "none" }
        }>
            <div class="mb-4">
                // r[impl users.collaboration]
                <a class="button is-primary" href=move || format!("/repo/{}/proposal/new", repo_id())>
                    "New Proposal"
                </a>
            </div>
            <Suspense fallback=move || {
                view! { <p>"Loading proposals..."</p> }
            }>
                {move || {
                    proposals_resource
                        .get()
                        .map(|result| match result {
                            Ok(proposal_list) => {
                                if proposal_list.is_empty() {
                                    view! {
                                        <p class="has-text-grey">"No proposals yet."</p>
                                    }
                                        .into_any()
                                } else {
                                    let rid = repo_id();
                                    view! {
                                        <div class="list">
                                            {proposal_list
                                                .into_iter()
                                                .map(|p| {
                                                    let display_title = p
                                                        .title
                                                        .unwrap_or_else(|| format!("Proposal #{}", p.id));
                                                    let badge_class = match p.status.as_str() {
                                                        "draft" => "tag is-warning",
                                                        "submitted" => "tag is-info",
                                                        "merged" => "tag is-success",
                                                        "closed" => "tag is-danger",
                                                        _ => "tag is-light",
                                                    };
                                                    view! {
                                                        <a
                                                            class="list-item"
                                                            href=format!(
                                                                "/repo/{}/proposal/{}",
                                                                rid,
                                                                p.id,
                                                            )
                                                        >
                                                            <span>{display_title}</span>
                                                            " "
                                                            <span class=badge_class>{p.status}</span>
                                                        </a>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </div>
                                    }
                                        .into_any()
                                }
                            }
                            Err(e) => {
                                view! {
                                    <div class="notification is-danger">
                                        {format!("Error: {e}")}
                                    </div>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        </div>
    }
}
