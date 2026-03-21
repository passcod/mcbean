use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_params_map};
use serde::{Deserialize, Serialize};

use crate::components::Editor;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalDetail {
    pub id: i32,
    pub repository_id: i32,
    pub title: Option<String>,
    pub status: String,
    // r[impl proposal.git.exposure]
    // branch_name deliberately excluded from client-facing DTO
    pub spec_content: String,
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
        use crate::db::schema::{proposals, spec_files, specs};

        let proposal = proposals::table
            .find(proposal_id)
            .select((
                proposals::id,
                proposals::repository_id,
                proposals::title,
                proposals::status,
            ))
            .first::<(i32, i32, Option<String>, String)>(conn)?;

        // r[impl repo.multi-spec]
        // Load all spec files for the repository, ordered for stable display.
        let contents: Vec<String> = spec_files::table
            .inner_join(specs::table)
            .filter(specs::repository_id.eq(proposal.1))
            .order((specs::name.asc(), spec_files::path.asc()))
            .select(spec_files::content)
            .load(conn)?;

        Ok(ProposalDetail {
            id: proposal.0,
            repository_id: proposal.1,
            title: proposal.2,
            status: proposal.3,
            spec_content: contents.join("\n\n"),
        })
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
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

#[component]
pub fn NewProposalPage() -> impl IntoView {
    let params = use_params_map();
    let repo_id = move || {
        params
            .read()
            .get("repo_id")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    };

    let title = RwSignal::new(String::new());
    let navigate = use_navigate();
    let create_action = Action::new(move |_: &()| {
        let t = title.get();
        let rid = repo_id();
        async move { create_proposal(rid, t).await }
    });

    Effect::new(move |_| {
        if let Some(Ok(new_id)) = create_action.value().get() {
            navigate(
                &format!("/repo/{}/proposal/{}", repo_id(), new_id),
                Default::default(),
            );
        }
    });

    view! {
        <h1 class="title">"New Proposal"</h1>

        {move || {
            create_action
                .value()
                .get()
                .and_then(|r| r.err())
                .map(|e| {
                    view! {
                        <div class="notification is-danger">{format!("Error: {e}")}</div>
                    }
                })
        }}

        <div class="box">
            <div class="field">
                // r[impl proposal.create.dismiss]
                <label class="label">"Title (optional)"</label>
                <div class="control">
                    <input
                        class="input"
                        type="text"
                        placeholder="Proposal title"
                        prop:value=move || title.get()
                        on:input=move |ev| {
                            title.set(event_target_value(&ev));
                        }
                    />
                </div>
            </div>

            <div class="field">
                <div class="control">
                    <button
                        class="button is-primary"
                        on:click=move |_| {
                            create_action.dispatch(());
                        }
                        disabled=move || create_action.pending().get()
                    >
                        {move || {
                            if create_action.pending().get() {
                                "Creating..."
                            } else {
                                "Create Proposal"
                            }
                        }}
                    </button>
                </div>
            </div>
        </div>
    }
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

    let editing_title = RwSignal::new(false);
    let title_draft = RwSignal::new(String::new());
    let editing_content = RwSignal::new(false);

    let save_title_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        let new_title = title_draft.get();
        async move { update_proposal_title(pid, new_title).await }
    });

    view! {
        <Suspense fallback=move || {
            view! { <p>"Loading proposal..."</p> }
        }>
            {move || {
                proposal
                    .get()
                    .map(|result| {
                        match result {
                            Ok(p) => {
                                let display_title = p
                                    .title
                                    .clone()
                                    .unwrap_or_else(|| format!("Proposal #{}", p.id));
                                let content = p.spec_content.clone();
                                let status = p.status.clone();
                                let initial_title = display_title.clone();
                                view! {
                                    <div class="level">
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
                                                                            title_draft.set(event_target_value(&ev));
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
                                                                        on:click=move |_| editing_title.set(false)
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
                                                            <h1 class="title">{t.clone()}</h1>
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
                                                <span class="tag is-info">{status.clone()}</span>
                                            </div>
                                        </div>
                                    </div>

                                    <div class="content">
                                        // r[impl edit.availability]
                                        // r[impl users.collaboration]
                                        <Show
                                            when=move || editing_content.get()
                                            fallback={
                                                let c = content.clone();
                                                move || {
                                                    let c2 = c.clone();
                                                    view! {
                                                        <div class="box">
                                                            <pre>{c2.clone()}</pre>
                                                        </div>
                                                        <button
                                                            class="button is-small is-light"
                                                            on:click=move |_| editing_content.set(true)
                                                        >
                                                            "Edit Content"
                                                        </button>
                                                    }
                                                }
                                            }
                                        >
                                            {
                                                let c = content.clone();
                                                view! {
                                                    // r[impl edit.rule-text]
                                                    <Editor
                                                        content=c.clone()
                                                        on_save=Callback::new(move |_new_content: String| {
                                                            // TODO: save content changes via server function
                                                            editing_content.set(false);
                                                        })
                                                        on_cancel=Callback::new(move |()| {
                                                            editing_content.set(false);
                                                        })
                                                    />
                                                }
                                            }
                                        </Show>
                                    </div>
                                }
                                    .into_any()
                            }
                            Err(e) => {
                                let msg = format!("Error loading proposal: {e}");
                                view! { <p class="has-text-danger">{msg}</p> }.into_any()
                            }
                        }
                    })
            }}
        </Suspense>
    }
}
