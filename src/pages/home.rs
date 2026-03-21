use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryInfo {
    pub id: i32,
    pub github_url: String,
    pub owner: String,
    pub name: String,
}

#[server]
pub async fn list_repositories() -> Result<Vec<RepositoryInfo>, ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(|conn| {
        use crate::db::schema::repositories::dsl::*;
        repositories
            .select((id, github_url, owner, name))
            .load::<(i32, String, String, String)>(conn)
            .map(|rows| {
                rows.into_iter()
                    .map(|(rid, url, o, n)| RepositoryInfo {
                        id: rid,
                        github_url: url,
                        owner: o,
                        name: n,
                    })
                    .collect()
            })
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e| ServerFnError::new(format!("{e}")))
}

#[server]
pub async fn add_repository(github_url: String) -> Result<RepositoryInfo, ServerFnError> {
    use diesel::prelude::*;

    let parts: Vec<&str> = github_url.trim_end_matches('/').rsplit('/').collect();
    if parts.len() < 2 {
        return Err(ServerFnError::new(
            "Invalid GitHub URL: expected owner/name in path",
        ));
    }
    let repo_name = parts[0].to_string();
    let repo_owner = parts[1].to_string();

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let url = github_url.clone();
    conn.interact(move |conn| {
        use crate::db::schema::repositories;
        diesel::insert_into(repositories::table)
            .values((
                repositories::github_url.eq(&url),
                repositories::owner.eq(&repo_owner),
                repositories::name.eq(&repo_name),
            ))
            .returning((
                repositories::id,
                repositories::github_url,
                repositories::owner,
                repositories::name,
            ))
            .get_result::<(i32, String, String, String)>(conn)
            .map(|(rid, u, o, n)| RepositoryInfo {
                id: rid,
                github_url: u,
                owner: o,
                name: n,
            })
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e| ServerFnError::new(format!("{e}")))
}

#[component]
pub fn HomePage() -> impl IntoView {
    let repos = Resource::new(|| (), |_| list_repositories());
    let add_action = ServerAction::<AddRepository>::new();
    let show_modal = RwSignal::new(false);
    let url_input = RwSignal::new(String::new());

    Effect::new(move || {
        if add_action.version().get() > 0 {
            show_modal.set(false);
            url_input.set(String::new());
            repos.refetch();
        }
    });

    view! {
        <section class="hero is-primary is-medium">
            <div class="hero-body">
                <p class="title">"McBean"</p>
                <p class="subtitle">"View, edit, and propose changes to Tracey spec files"</p>
            </div>
        </section>

        <section class="section">
            <div class="level">
                <div class="level-left">
                    <h2 class="title is-4">"Repositories"</h2>
                </div>
                <div class="level-right">
                    <button
                        class="button is-primary"
                        on:click=move |_| show_modal.set(true)
                    >
                        "Add Repository"
                    </button>
                </div>
            </div>

            <Suspense fallback=move || view! { <p>"Loading repositories..."</p> }>
                {move || {
                    repos.get().map(|result| match result {
                        Ok(repos) => {
                            if repos.is_empty() {
                                view! {
                                    <div class="notification is-info is-light">
                                        "No repositories connected yet. Click \"Add Repository\" to get started."
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="columns is-multiline">
                                        {repos.into_iter().map(|repo| {
                                            let href = format!("/repo/{}", repo.id);
                                            let url = repo.github_url;
                                            let url_href = url.clone();
                                            let label = format!("{}/{}", repo.owner, repo.name);
                                            view! {
                                                <div class="column is-one-third">
                                                    <div class="card">
                                                        <div class="card-content">
                                                            <p class="title is-5">
                                                                {label}
                                                            </p>
                                                            <p class="subtitle is-6">
                                                                <a href={url_href} target="_blank">
                                                                    {url}
                                                                </a>
                                                            </p>
                                                        </div>
                                                        <footer class="card-footer">
                                                            <a href={href} class="card-footer-item">"Open"</a>
                                                        </footer>
                                                    </div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                        }
                        Err(e) => view! {
                            <div class="notification is-danger">
                                {format!("Error loading repositories: {e}")}
                            </div>
                        }.into_any(),
                    })
                }}
            </Suspense>
        </section>

        <div class=move || if show_modal.get() { "modal is-active" } else { "modal" }>
            <div class="modal-background" on:click=move |_| show_modal.set(false)></div>
            <div class="modal-card">
                <header class="modal-card-head">
                    <p class="modal-card-title">"Add Repository"</p>
                    <button
                        class="delete"
                        aria-label="close"
                        on:click=move |_| show_modal.set(false)
                    ></button>
                </header>
                <section class="modal-card-body">
                    <ActionForm action=add_action>
                        <div class="field">
                            <label class="label">"GitHub URL"</label>
                            <div class="control">
                                <input
                                    class="input"
                                    type="text"
                                    name="github_url"
                                    placeholder="https://github.com/owner/repo"
                                    prop:value=move || url_input.get()
                                    on:input=move |ev| {
                                        url_input.set(event_target_value(&ev));
                                    }
                                />
                            </div>
                        </div>
                        <div class="field">
                            <div class="control">
                                <button type="submit" class="button is-primary">
                                    "Add"
                                </button>
                            </div>
                        </div>
                    </ActionForm>
                </section>
            </div>
        </div>
    }
}
