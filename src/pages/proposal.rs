use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

use crate::components::Editor;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalDetail {
    pub id: i32,
    pub repository_id: i32,
    pub spec_id: i32,
    pub title: Option<String>,
    pub status: String,
    pub spec_content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecSummary {
    pub id: i32,
    pub name: String,
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
        use crate::db::schema::{proposals, spec_files};

        let proposal = proposals::table
            .find(proposal_id)
            .select((
                proposals::id,
                proposals::repository_id,
                proposals::spec_id,
                proposals::title,
                proposals::status,
            ))
            .first::<(i32, i32, i32, Option<String>, String)>(conn)?;

        let content = spec_files::table
            .filter(spec_files::spec_id.eq(proposal.2))
            .select(spec_files::content)
            .first::<String>(conn)
            .unwrap_or_default();

        Ok(ProposalDetail {
            id: proposal.0,
            repository_id: proposal.1,
            spec_id: proposal.2,
            title: proposal.3,
            status: proposal.4,
            spec_content: content,
        })
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

#[server]
pub async fn create_proposal(
    repo_id: i32,
    spec_id: i32,
    title: String,
) -> Result<i32, ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let branch_name = format!(
        "mcbean/proposal-{}",
        hex::encode(&rand::random::<[u8; 4]>())
    );
    let title_val = if title.is_empty() {
        None
    } else {
        Some(title.clone())
    };
    let title_is_user = title_val.is_some();

    conn.interact(move |conn| {
        use crate::db::schema::proposals;

        diesel::insert_into(proposals::table)
            .values(&crate::db::models::NewProposal {
                repository_id: repo_id,
                spec_id,
                title: title_val,
                title_is_user_supplied: Some(title_is_user),
                branch_name,
                status: Some("draft".to_string()),
                created_by: 1, // TODO: get from auth context
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
                proposals::title.eq(Some(&title)),
                proposals::title_is_user_supplied.eq(true),
            ))
            .execute(conn)?;
        Ok(())
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

#[server]
pub async fn list_specs_for_repo(repo_id: i32) -> Result<Vec<SpecSummary>, ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::specs;

        specs::table
            .filter(specs::repository_id.eq(repo_id))
            .select((specs::id, specs::name))
            .load::<(i32, String)>(conn)
            .map(|rows| {
                rows.into_iter()
                    .map(|(id, name)| SpecSummary { id, name })
                    .collect()
            })
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

    let specs = Resource::new(repo_id, list_specs_for_repo);

    let title = RwSignal::new(String::new());
    let selected_spec = RwSignal::new(Option::<i32>::None);
    let create_action = Action::new(move |_: &()| {
        let t = title.get();
        let rid = repo_id();
        let sid = selected_spec.get().unwrap_or(0);
        async move { create_proposal(rid, sid, t).await }
    });

    view! {
        <h1 class="title">"New Proposal"</h1>

        <div class="box">
            <div class="field">
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
                <label class="label">"Spec"</label>
                <div class="control">
                    <div class="select">
                        <Suspense fallback=move || {
                            view! { <select disabled><option>"Loading..."</option></select> }
                        }>
                            {move || {
                                specs
                                    .get()
                                    .map(|result| {
                                        match result {
                                            Ok(spec_list) => {
                                                view! {
                                                    <select on:change=move |ev| {
                                                        let val = event_target_value(&ev);
                                                        selected_spec.set(val.parse::<i32>().ok());
                                                    }>
                                                        <option value="">"Select a spec..."</option>
                                                        {spec_list
                                                            .into_iter()
                                                            .map(|s| {
                                                                let id_str = s.id.to_string();
                                                                view! { <option value=id_str>{s.name}</option> }
                                                            })
                                                            .collect_view()}
                                                    </select>
                                                }
                                                    .into_any()
                                            }
                                            Err(_) => {
                                                view! {
                                                    <select disabled>
                                                        <option>"Failed to load specs"</option>
                                                    </select>
                                                }
                                                    .into_any()
                                            }
                                        }
                                    })
                            }}
                        </Suspense>
                    </div>
                </div>
            </div>

            <div class="field">
                <div class="control">
                    <button
                        class="button is-primary"
                        on:click=move |_| {
                            create_action.dispatch(());
                        }
                        disabled=move || selected_spec.get().is_none()
                    >
                        "Create Proposal"
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
