use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryInfo {
    pub id: i32,
    pub github_url: String,
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddRepoResult {
    pub repo: RepositoryInfo,
    pub specs_found: Vec<String>,
}

#[server]
pub async fn list_repositories() -> Result<Vec<RepositoryInfo>, ServerFnError> {
    use diesel::prelude::*;

    tracing::info!("listing all repositories");

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool error: {e}")))?;

    let result: Vec<RepositoryInfo> = conn
        .interact(|conn| {
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
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e| ServerFnError::new(format!("query error: {e}")))?;

    tracing::info!(count = result.len(), "listed repositories");
    Ok(result)
}

// r[impl repo.connect]
#[server]
pub async fn add_repository(github_url: String) -> Result<AddRepoResult, ServerFnError> {
    use diesel::prelude::*;

    tracing::info!(%github_url, "add_repository called");

    let trimmed = github_url.trim_end_matches('/');
    let parts: Vec<&str> = trimmed.rsplit('/').collect();
    if parts.len() < 2 {
        tracing::warn!(%github_url, "invalid GitHub URL: cannot extract owner/name");
        return Err(ServerFnError::new(
            "Invalid GitHub URL: expected owner/name in path",
        ));
    }
    let repo_name = parts[0].to_string();
    let repo_owner = parts[1].to_string();

    tracing::info!(owner = %repo_owner, name = %repo_name, "parsed repository coordinates");

    let github = use_context::<crate::github::GitHubClient>()
        .ok_or_else(|| ServerFnError::new("No GitHub client available"))?;

    // Fetch repo metadata to confirm it exists and get the default branch
    let metadata = github
        .get_repo_metadata(&repo_owner, &repo_name)
        .await
        .map_err(|e| {
            tracing::error!(
                owner = %repo_owner,
                name = %repo_name,
                error = %e,
                "failed to fetch repository metadata from GitHub"
            );
            ServerFnError::new(format!("GitHub API error fetching repo metadata: {e}"))
        })?;

    tracing::info!(
        default_branch = %metadata.default_branch,
        "fetched repo metadata from GitHub"
    );

    // Get the HEAD commit SHA for storing with spec files
    let commit_sha = github
        .get_branch_head_sha(&repo_owner, &repo_name, &metadata.default_branch)
        .await
        .map_err(|e| {
            tracing::error!(
                branch = %metadata.default_branch,
                error = %e,
                "failed to fetch branch HEAD SHA"
            );
            ServerFnError::new(format!("GitHub API error fetching branch HEAD: {e}"))
        })?;

    tracing::info!(%commit_sha, "resolved HEAD commit");

    // Try to fetch .config/tracey/config.styx
    let config_path = ".config/tracey/config.styx";
    let config_result = github
        .get_file_contents(
            &repo_owner,
            &repo_name,
            config_path,
            &metadata.default_branch,
        )
        .await;

    let spec_defs = match config_result {
        Ok(fetched) => {
            tracing::info!(
                path = config_path,
                content_len = fetched.content.len(),
                "fetched tracey config"
            );
            facet_styx::from_str::<tracey_config::Config>(&fetched.content)
                .map_err(|e| {
                    tracing::error!(error = %e, "failed to parse tracey config");
                    ServerFnError::new(format!("Failed to parse tracey config: {e}"))
                })?
                .specs
        }
        Err(crate::github::GitHubError::NotFound { .. }) => {
            tracing::warn!(path = config_path, "tracey config not found in repository");
            return Err(ServerFnError::new(
                "No tracey configuration found at .config/tracey/config.styx — \
                 this repository does not appear to contain a Tracey spec",
            ));
        }
        Err(e) => {
            tracing::error!(
                path = config_path,
                error = %e,
                "failed to fetch tracey config"
            );
            return Err(ServerFnError::new(format!(
                "GitHub API error fetching tracey config: {e}"
            )));
        }
    };

    if spec_defs.is_empty() {
        tracing::warn!("tracey config was parsed but contained no spec definitions");
        return Err(ServerFnError::new(
            "Tracey config was found but contains no spec definitions",
        ));
    }

    tracing::info!(
        spec_count = spec_defs.len(),
        specs = ?spec_defs.iter().map(|s| &s.name).collect::<Vec<_>>(),
        "parsed tracey config"
    );

    // Fetch all spec files from GitHub (handles both literal paths and globs)
    let mut spec_files_by_spec: Vec<(String, Vec<(String, String)>)> = Vec::new();

    for spec_def in &spec_defs {
        tracing::info!(
            spec = %spec_def.name,
            patterns = ?spec_def.include,
            "resolving spec include patterns"
        );
        let fetched = github
            .resolve_include_patterns(
                &repo_owner,
                &repo_name,
                &metadata.default_branch,
                &spec_def.include,
            )
            .await
            .map_err(|e| {
                tracing::error!(
                    spec = %spec_def.name,
                    error = %e,
                    "failed to resolve spec include patterns"
                );
                ServerFnError::new(format!(
                    "Failed to fetch spec files for {}: {e}",
                    spec_def.name
                ))
            })?;

        tracing::info!(
            spec = %spec_def.name,
            file_count = fetched.len(),
            paths = ?fetched.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "resolved spec files"
        );

        let files = fetched.into_iter().map(|f| (f.path, f.content)).collect();
        spec_files_by_spec.push((spec_def.name.clone(), files));
    }

    // Verify that at least one spec has at least one file with tracey rule annotations
    let has_tracey_rules = spec_files_by_spec.iter().any(|(_, files)| {
        files.iter().any(|(_, content)| {
            content
                .as_bytes()
                .windows(2)
                .any(|w| w[0].is_ascii_lowercase() && w[1] == b'[')
        })
    });

    if !has_tracey_rules {
        tracing::warn!("no tracey rule annotations (r[...]) found in any spec file");
        return Err(ServerFnError::new(
            "The spec files listed in the tracey config do not contain any Tracey rule \
             annotations (r[...]). Are you sure this is a Tracey spec repository?",
        ));
    }

    // All validation passed — build the Loro snapshot before touching the DB
    // (async: uses marq to parse each file's Markdown into the tree).
    let doc = crate::components::loro_doc::build_doc_from_specs(&spec_files_by_spec).await;
    let loro_bytes = doc
        .export(loro::ExportMode::Snapshot)
        .map_err(|e| ServerFnError::new(format!("loro export: {e}")))?;

    let spec_names_for_db: Vec<String> = spec_files_by_spec
        .iter()
        .map(|(name, _)| name.clone())
        .collect();

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("pool error: {e}")))?;

    let default_branch = metadata.default_branch.clone();
    let url = github_url.clone();
    let owner_clone = repo_owner.clone();
    let name_clone = repo_name.clone();

    let result = conn
        .interact(move |conn| {
            use crate::db::schema::{repositories, spec_snapshots, specs};

            conn.transaction::<AddRepoResult, diesel::result::Error, _>(|conn| {
                // Insert the repository.
                let (rid, rurl, rowner, rname): (i32, String, String, String) =
                    diesel::insert_into(repositories::table)
                        .values((
                            repositories::github_url.eq(&url),
                            repositories::owner.eq(&owner_clone),
                            repositories::name.eq(&name_clone),
                            repositories::default_branch.eq(&default_branch),
                            repositories::last_synced_sha.eq(&commit_sha),
                        ))
                        .returning((
                            repositories::id,
                            repositories::github_url,
                            repositories::owner,
                            repositories::name,
                        ))
                        .get_result(conn)?;

                let repo_info = RepositoryInfo {
                    id: rid,
                    github_url: rurl,
                    owner: rowner,
                    name: rname,
                };

                // Insert spec name rows (used for navigation queries).
                for spec_name in &spec_names_for_db {
                    diesel::insert_into(specs::table)
                        .values((specs::repository_id.eq(rid), specs::name.eq(spec_name)))
                        .execute(conn)?;
                }

                // Store the Loro snapshot for this commit SHA.
                diesel::insert_into(spec_snapshots::table)
                    .values((
                        spec_snapshots::repository_id.eq(rid),
                        spec_snapshots::commit_sha.eq(&commit_sha),
                        spec_snapshots::loro_bytes.eq(&loro_bytes),
                    ))
                    .execute(conn)?;

                Ok(AddRepoResult {
                    repo: repo_info,
                    specs_found: spec_names_for_db.clone(),
                })
            })
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e| {
            tracing::error!(error = %e, "database transaction failed");
            ServerFnError::new(format!("Database error: {e}"))
        })?;

    tracing::info!(
        repo_id = result.repo.id,
        specs = ?result.specs_found,
        "repository added with specs"
    );

    Ok(result)
}

#[component]
pub fn HomePage() -> impl IntoView {
    let repos = Resource::new(|| (), |_| list_repositories());
    let add_action = ServerAction::<AddRepository>::new();
    let show_modal = RwSignal::new(false);
    let url_input = RwSignal::new(String::new());
    let error_message = RwSignal::new(Option::<String>::None);

    Effect::new(move || {
        if add_action.version().get() > 0 {
            match add_action.value().get() {
                Some(Ok(_)) => {
                    show_modal.set(false);
                    url_input.set(String::new());
                    error_message.set(None);
                    repos.refetch();
                }
                Some(Err(e)) => {
                    error_message.set(Some(e.to_string()));
                }
                None => {}
            }
        }
    });

    let is_submitting = move || add_action.pending().get();

    view! {
        <Title text="Repositories"/>
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
                        on:click=move |_| {
                            error_message.set(None);
                            show_modal.set(true);
                        }
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
            <div class="modal-background" on:click=move |_| {
                if !is_submitting() {
                    show_modal.set(false);
                }
            }></div>
            <div class="modal-card">
                <header class="modal-card-head">
                    <p class="modal-card-title">"Add Repository"</p>
                    <button
                        class="delete"
                        aria-label="close"
                        disabled=is_submitting
                        on:click=move |_| show_modal.set(false)
                    ></button>
                </header>
                <section class="modal-card-body">
                    {move || error_message.get().map(|msg| view! {
                        <div class="notification is-danger">
                            <button class="delete" on:click=move |_| error_message.set(None)></button>
                            {msg}
                        </div>
                    })}

                    // r[impl repo.connect]
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
                                    disabled=is_submitting
                                />
                            </div>
                            <p class="help">
                                "The repository must contain a Tracey configuration at "
                                <code>".config/tracey/config.styx"</code>
                            </p>
                        </div>
                        <div class="field">
                            <div class="control">
                                <button
                                    type="submit"
                                    class="button is-primary"
                                    class:is-loading=is_submitting
                                    disabled=is_submitting
                                >
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
