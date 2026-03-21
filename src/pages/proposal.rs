use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

use crate::components::{
    ChangelogSidebar, FinaliseFab, SpecBlock, SpecBlockEditor, SpecSidebar, blocks_to_sidebar_data,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalDetail {
    pub id: i32,
    pub repository_id: i32,
    pub title: Option<String>,
    pub status: String,
    // r[impl proposal.git.exposure]
    // branch_name deliberately excluded from client-facing DTO
}

#[server]
pub async fn get_proposal(proposal_id: i32) -> Result<ProposalDetail, ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::proposals;

        let (id, repository_id, title, status) = proposals::table
            .find(proposal_id)
            .select((
                proposals::id,
                proposals::repository_id,
                proposals::title,
                proposals::status,
            ))
            .first::<(i32, i32, Option<String>, String)>(conn)?;

        Ok(ProposalDetail {
            id,
            repository_id,
            title,
            status,
        })
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

// r[impl edit.availability]
// r[impl edit.rule-text]
// r[impl edit.add-rule]
// r[impl edit.add-section]
// r[impl edit.reorder]
// r[impl edit.delete]
#[server]
pub async fn get_proposal_blocks(proposal_id: i32) -> Result<Vec<SpecBlock>, ServerFnError> {
    use diesel::prelude::*;

    use crate::components::spec_block_editor::parse_blocks_from_content;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let (repository_id, latest_snapshot): (i32, Option<String>) = conn
        .interact(move |conn| {
            use crate::db::schema::{proposal_changes, proposals};

            let repo_id: i32 = proposals::table
                .find(proposal_id)
                .select(proposals::repository_id)
                .first(conn)?;

            // r[impl edit.history]
            // The latest change's snapshot is the current content of the proposal.
            let snapshot: Option<String> = proposal_changes::table
                .filter(proposal_changes::proposal_id.eq(proposal_id))
                .order(proposal_changes::id.desc())
                .select(proposal_changes::content_snapshot)
                .first(conn)
                .optional()?;

            Ok::<_, diesel::result::Error>((repo_id, snapshot))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query error: {e}")))?;

    let content = if let Some(snapshot) = latest_snapshot {
        snapshot
    } else {
        // No edits recorded yet — serve the base spec from the main branch.
        // r[impl repo.multi-spec]
        // r[impl repo.multi-file]
        conn.interact(move |conn| {
            use crate::db::schema::{spec_files, specs};

            let contents: Vec<String> = spec_files::table
                .inner_join(specs::table)
                .filter(specs::repository_id.eq(repository_id))
                .order((specs::name.asc(), spec_files::path.asc()))
                .select(spec_files::content)
                .load(conn)?;

            Ok::<_, diesel::result::Error>(contents.join("\n\n"))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query error: {e}")))?
    };

    Ok(parse_blocks_from_content(&content).await)
}

// r[impl proposal.diff.semantic]
#[server]
pub async fn get_base_blocks(proposal_id: i32) -> Result<Vec<SpecBlock>, ServerFnError> {
    use diesel::prelude::*;

    use crate::components::spec_block_editor::parse_blocks_from_content;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let content = conn
        .interact(move |conn| {
            use crate::db::schema::{proposals, spec_files, specs};

            // r[impl repo.multi-spec]
            // r[impl repo.multi-file]
            let repository_id: i32 = proposals::table
                .find(proposal_id)
                .select(proposals::repository_id)
                .first(conn)?;

            let contents: Vec<String> = spec_files::table
                .inner_join(specs::table)
                .filter(specs::repository_id.eq(repository_id))
                .order((specs::name.asc(), spec_files::path.asc()))
                .select(spec_files::content)
                .load(conn)?;

            Ok::<_, diesel::result::Error>(contents.join("\n\n"))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact error: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query error: {e}")))?;

    Ok(parse_blocks_from_content(&content).await)
}

#[server]
pub async fn create_proposal(repo_id: i32, title: String) -> Result<i32, ServerFnError> {
    use diesel::prelude::*;

    // r[impl users.identity]
    let user_id = crate::auth::get_or_create_user_id().await?;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    // r[impl proposal.git.backing]
    let branch_name = format!("mcbean/proposal-{}", hex::encode(rand::random::<[u8; 4]>()));
    // r[impl proposal.create.dismiss]
    let title_val = if title.is_empty() {
        None
    } else {
        Some(title.clone())
    };
    // r[impl proposal.title.user-priority]
    let title_is_user = title_val.is_some();

    conn.interact(move |conn| {
        use crate::db::schema::proposals;

        diesel::insert_into(proposals::table)
            .values(&crate::db::models::NewProposal {
                repository_id: repo_id,
                title: title_val,
                title_is_user_supplied: Some(title_is_user),
                branch_name,
                // r[impl lifecycle.drafting]
                status: None,
                created_by: user_id,
            })
            .returning(proposals::id)
            .get_result::<i32>(conn)
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

#[server]
pub async fn update_proposal_title(proposal_id: i32, title: String) -> Result<(), ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::proposals;

        diesel::update(proposals::table.find(proposal_id))
            .set((
                // r[impl proposal.title.editable]
                proposals::title.eq(Some(&title)),
                // r[impl proposal.title.user-priority]
                proposals::title_is_user_supplied.eq(true),
            ))
            .execute(conn)?;
        Ok(())
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

// r[impl proposal.submit]
// r[impl ids.finalise-phase]
#[server]
pub async fn finalise_proposal(_proposal_id: i32) -> Result<(), ServerFnError> {
    // TODO: run LLM finalisation pass, present ID review to user, create PR.
    Err(ServerFnError::new("Finalisation not yet implemented"))
}

#[component]
pub fn ProposalPage() -> impl IntoView {
    let params = use_params_map();
    let proposal_id = move || {
        params
            .read()
            .get("proposal_id")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    };

    let proposal = Resource::new(proposal_id, get_proposal);
    let blocks_resource = Resource::new(proposal_id, get_proposal_blocks);
    let base_blocks_resource = Resource::new(proposal_id, get_base_blocks);

    let editing_title = RwSignal::new(false);
    let title_draft = RwSignal::new(String::new());

    let save_title_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        let new_title = title_draft.get();
        async move { update_proposal_title(pid, new_title).await }
    });

    let finalise_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        async move { finalise_proposal(pid).await }
    });

    view! {
        <Suspense fallback=move || view! { <p>"Loading proposal…"</p> }>
            {move || {
                proposal
                    .get()
                    .map(|result| match result {
                        Ok(p) => {
                            let display_title = p
                                .title
                                .clone()
                                .unwrap_or_else(|| format!("Proposal #{}", p.id));
                            let status = p.status.clone();
                            let is_drafting = status == "drafting";
                            let initial_title = display_title.clone();
                            let sidebar_title = display_title.clone();

                            view! {
                                <Title text=display_title.clone()/>
                                // ── Title row ────────────────────────────────
                                <div class="level mb-4">
                                    <div class="level-left">
                                        <div class="level-item">
                                            // r[impl proposal.title.editable]
                                            // r[impl proposal.git.exposure]
                                            <Show
                                                when=move || !editing_title.get()
                                                fallback=move || {
                                                    view! {
                                                        <div class="field has-addons">
                                                            <div class="control">
                                                                <input
                                                                    class="input"
                                                                    type="text"
                                                                    prop:value=move || title_draft.get()
                                                                    on:input=move |ev| {
                                                                        title_draft
                                                                            .set(event_target_value(&ev));
                                                                    }
                                                                    on:keydown=move |ev| {
                                                                        if ev.key() == "Enter" {
                                                                            save_title_action.dispatch(());
                                                                            editing_title.set(false);
                                                                        } else if ev.key() == "Escape" {
                                                                            editing_title.set(false);
                                                                        }
                                                                    }
                                                                />
                                                            </div>
                                                            <div class="control">
                                                                <button
                                                                    class="button is-success"
                                                                    on:click=move |_| {
                                                                        save_title_action.dispatch(());
                                                                        editing_title.set(false);
                                                                    }
                                                                >
                                                                    "Save"
                                                                </button>
                                                            </div>
                                                            <div class="control">
                                                                <button
                                                                    class="button is-light"
                                                                    on:click=move |_| editing_title
                                                                        .set(false)
                                                                >
                                                                    "Cancel"
                                                                </button>
                                                            </div>
                                                        </div>
                                                    }
                                                }
                                            >
                                                {
                                                    let t = display_title.clone();
                                                    let it = initial_title.clone();
                                                    view! {
                                                        <h1 class="title mb-0">{t}</h1>
                                                        <button
                                                            class="button is-small is-light ml-2"
                                                            on:click=move |_| {
                                                                title_draft.set(it.clone());
                                                                editing_title.set(true);
                                                            }
                                                        >
                                                            "Edit"
                                                        </button>
                                                    }
                                                }
                                            </Show>
                                        </div>
                                    </div>
                                    <div class="level-right">
                                        <div class="level-item">
                                            <span class=format!(
                                                "tag {}",
                                                match status.as_str() {
                                                    "drafting" => "is-warning",
                                                    "in_progress" => "is-info",
                                                    "merged" => "is-success",
                                                    "abandoned" => "is-danger",
                                                    _ => "is-light",
                                                },
                                            )>{status.clone()}</span>
                                        </div>
                                    </div>
                                </div>



                                // ── Finalise error feedback ───────────────────
                                {move || {
                                    finalise_action
                                        .value()
                                        .get()
                                        .and_then(|r: Result<(), _>| r.err())
                                        .map(|e| {
                                            view! {
                                                <div class="notification is-danger is-light mb-4">
                                                    {format!("Finalise failed: {e}")}
                                                </div>
                                            }
                                        })
                                }}

                                // ── Spec content ──────────────────────────────
                                // r[impl edit.availability]
                                // r[impl users.collaboration]
                                <Suspense fallback=move || {
                                    view! { <p class="has-text-grey">"Loading spec…"</p> }
                                }>
                                    {move || {
                                        blocks_resource
                                            .get()
                                            .map(|result: Result<Vec<SpecBlock>, _>| match result {
                                                Ok(blocks) if is_drafting => {
                                                                    // r[impl edit.rule-text]
                                                                    // r[impl edit.add-rule]
                                                                    // r[impl edit.add-section]
                                                                    // r[impl edit.reorder]
                                                                    // r[impl edit.delete]
                                                                    let Some(base_result) =
                                                                        base_blocks_resource.get()
                                                                    else {
                                                                        return view! {
                                                                            <p class="has-text-grey">
                                                                                "Loading\u{2026}"
                                                                            </p>
                                                                        }
                                                                        .into_any();
                                                                    };
                                                                    let initial_blocks =
                                                                        base_result.unwrap_or_else(
                                                                            |_| blocks.clone(),
                                                                        );
                                                                    let blocks_signal =
                                                                        RwSignal::new(blocks);
                                                                    let (outline, search_entries) =
                                                                        blocks_to_sidebar_data(
                                                                            &blocks_signal.get_untracked(),
                                                                            &sidebar_title,
                                                                        );
                                                                    view! {
                                                                        <div style="display: flex; align-items: flex-start; margin: 0 -1.5rem;">
                                                                            <SpecSidebar outline=outline search_entries=search_entries />
                                                                            <div style="flex: 1; min-width: 0; padding: 0 1.5rem;">
                                                                                <SpecBlockEditor
                                                                                    blocks_signal=blocks_signal
                                                                                    proposal_id=p.id
                                                                                />
                                                                            </div>
                                                                            // r[impl proposal.diff.semantic]
                                                                            <ChangelogSidebar
                                                                                initial_blocks=initial_blocks
                                                                                blocks=Signal::from(blocks_signal)
                                                                            />
                                                                        </div>
                                                                    }
                                                                    .into_any()
                                                                }
                                                                Ok(blocks) => {
                                                                    let title = sidebar_title.clone();
                                                                    let (outline, search_entries) =
                                                                        blocks_to_sidebar_data(&blocks, &title);
                                                                    view! {
                                                                        <div style="display: flex; align-items: flex-start; margin: 0 -1.5rem;">
                                                                            <SpecSidebar outline=outline search_entries=search_entries />
                                                                            <div style="flex: 1; min-width: 0; padding: 0 1.5rem;">
                                                                                <div class="spec-readonly">
                                                                                    {blocks
                                                                                        .into_iter()
                                                                                        .map(|b| {
                                                                                            let html = b.html.clone();
                                                                                            let text = b
                                                                                                .edit_text()
                                                                                                .to_owned();
                                                                                            view! {
                                                                                                <div class="content mb-3">
                                                                                                    {if html.is_empty() {
                                                                                                        view! { <p>{text}</p> }
                                                                                                            .into_any()
                                                                                                    } else {
                                                                                                        view! {
                                                                                                            <div inner_html=html />
                                                                                                        }
                                                                                                            .into_any()
                                                                                                    }}
                                                                                                </div>
                                                                                            }
                                                                                        })
                                                                                        .collect::<Vec<_>>()}
                                                                                </div>
                                                                            </div>
                                                                        </div>
                                                                    }
                                                                    .into_any()
                                                                }
                                                Err(e) => view! {
                                                    <div class="notification is-danger">
                                                        {format!("Error loading spec: {e}")}
                                                    </div>
                                                }
                                                .into_any(),
                                            })
                                    }}
                                </Suspense>

                                // r[impl proposal.submit]
                                <Show when=move || is_drafting>
                                    <FinaliseFab
                                        on_finalise=Callback::new(move |_| {
                                            finalise_action.dispatch(());
                                        })
                                        pending=finalise_action.pending().get()
                                    />
                                </Show>
                            }
                            .into_any()
                        }
                        Err(e) => view! {
                            <p class="has-text-danger">
                                {format!("Error loading proposal: {e}")}
                            </p>
                        }
                        .into_any(),
                    })
            }}
        </Suspense>
    }
}
