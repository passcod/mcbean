use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use serde::{Deserialize, Serialize};

use crate::components::{
    ChangelogSidebar, FinaliseFab, FinalisingView, RevertOp, SpecBlock, SpecBlockEditor,
    SpecSidebar, blocks_to_sidebar_data,
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
#[server]
pub async fn get_proposal_blocks(proposal_id: i32) -> Result<Vec<SpecBlock>, ServerFnError> {
    use diesel::prelude::*;

    use crate::components::loro_doc::{loro_doc_to_blocks, reconstruct_doc};

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let (base_bytes, update_rows) = conn
        .interact(move |conn| {
            use crate::db::schema::{proposal_loro_updates, spec_snapshots};

            use crate::components::spec_block_editor::resolve_base_snapshot_id;

            let sid = match resolve_base_snapshot_id(proposal_id, conn) {
                Ok(sid) => sid,
                Err(diesel::result::Error::NotFound) => {
                    return Ok::<_, diesel::result::Error>((Vec::new(), Vec::new()));
                }
                Err(e) => return Err(e),
            };

            let base_bytes: Vec<u8> = spec_snapshots::table
                .find(sid)
                .select(spec_snapshots::loro_bytes)
                .first(conn)?;

            let update_rows: Vec<Vec<u8>> = proposal_loro_updates::table
                .filter(proposal_loro_updates::proposal_id.eq(proposal_id))
                .order(proposal_loro_updates::id.asc())
                .select(proposal_loro_updates::update_bytes)
                .load(conn)?;

            Ok((base_bytes, update_rows))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    if base_bytes.is_empty() {
        return Ok(Vec::new());
    }

    let doc = reconstruct_doc(&base_bytes, &update_rows)
        .map_err(|e| ServerFnError::new(format!("reconstruct: {e}")))?;

    Ok(loro_doc_to_blocks(&doc))
}

// r[impl proposal.diff.semantic]
#[server]
pub async fn get_base_blocks(proposal_id: i32) -> Result<Vec<SpecBlock>, ServerFnError> {
    use diesel::prelude::*;

    use crate::components::loro_doc::loro_doc_to_blocks;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let base_bytes: Vec<u8> = conn
        .interact(move |conn| {
            use crate::db::schema::spec_snapshots;

            use crate::components::spec_block_editor::resolve_base_snapshot_id;

            let sid = match resolve_base_snapshot_id(proposal_id, conn) {
                Ok(sid) => sid,
                Err(diesel::result::Error::NotFound) => {
                    return Ok::<_, diesel::result::Error>(Vec::new());
                }
                Err(e) => return Err(e),
            };

            spec_snapshots::table
                .find(sid)
                .select(spec_snapshots::loro_bytes)
                .first(conn)
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    if base_bytes.is_empty() {
        return Ok(Vec::new());
    }

    let doc = loro::LoroDoc::new();
    doc.import(&base_bytes)
        .map_err(|e| ServerFnError::new(format!("loro import: {e}")))?;
    Ok(loro_doc_to_blocks(&doc))
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

// r[impl lifecycle.finalising]
#[server]
pub async fn finalise_proposal(proposal_id: i32) -> Result<(), ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    conn.interact(move |conn| {
        use crate::db::schema::proposals;

        let current_status: String = proposals::table
            .find(proposal_id)
            .select(proposals::status)
            .first(conn)?;

        if current_status != "drafting" {
            return Err(diesel::result::Error::QueryBuilderError(
                format!("cannot finalise a proposal in '{current_status}' state").into(),
            ));
        }

        diesel::update(proposals::table.find(proposal_id))
            .set(proposals::status.eq("finalising"))
            .execute(conn)?;

        Ok(())
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

/// Return the proposal to drafting from the finalising screen.
// r[impl lifecycle.finalising]
#[server]
pub async fn unfinalise_proposal(proposal_id: i32) -> Result<(), ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    conn.interact(move |conn| {
        use crate::db::schema::proposals;

        let current_status: String = proposals::table
            .find(proposal_id)
            .select(proposals::status)
            .first(conn)?;

        if current_status != "finalising" {
            return Err(diesel::result::Error::QueryBuilderError(
                format!("cannot unfinalise a proposal in '{current_status}' state").into(),
            ));
        }

        diesel::update(proposals::table.find(proposal_id))
            .set(proposals::status.eq("drafting"))
            .execute(conn)?;

        Ok(())
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))
}

/// Submit the proposal: create a branch, commit spec files, open a PR, send Slack.
// r[impl lifecycle.submitted]
// r[impl proposal.submit]
// r[impl ids.finalise-phase]
#[server]
pub async fn submit_proposal(proposal_id: i32) -> Result<(), ServerFnError> {
    use diesel::prelude::*;

    use crate::components::loro_doc::{doc_to_markdown_files, reconstruct_doc};
    use crate::github::FileToCommit;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let github = use_context::<crate::github::GitHubClient>()
        .ok_or_else(|| ServerFnError::new("No GitHub client"))?;

    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    // Load proposal + repo info in one interact call.
    let (
        branch_name,
        title,
        repo_owner,
        repo_name,
        default_branch,
        slack_url,
        base_bytes,
        update_rows,
    ) = conn
        .interact(move |conn| {
            use crate::components::spec_block_editor::resolve_base_snapshot_id;
            use crate::db::schema::{
                proposal_loro_updates, proposals, repositories, spec_snapshots,
            };

            let (branch_name, title, status, repo_id): (String, Option<String>, String, i32) =
                proposals::table
                    .find(proposal_id)
                    .select((
                        proposals::branch_name,
                        proposals::title,
                        proposals::status,
                        proposals::repository_id,
                    ))
                    .first(conn)?;

            if status != "finalising" {
                return Err(diesel::result::Error::QueryBuilderError(
                    format!("cannot submit a proposal in '{status}' state").into(),
                ));
            }

            let (repo_owner, repo_name, default_branch, slack_url): (
                String,
                String,
                String,
                Option<String>,
            ) = repositories::table
                .find(repo_id)
                .select((
                    repositories::owner,
                    repositories::name,
                    repositories::default_branch,
                    repositories::slack_webhook_url,
                ))
                .first(conn)?;

            let sid = resolve_base_snapshot_id(proposal_id, conn)?;

            let base_bytes: Vec<u8> = spec_snapshots::table
                .find(sid)
                .select(spec_snapshots::loro_bytes)
                .first(conn)?;

            let update_rows: Vec<Vec<u8>> = proposal_loro_updates::table
                .filter(proposal_loro_updates::proposal_id.eq(proposal_id))
                .order(proposal_loro_updates::id.asc())
                .select(proposal_loro_updates::update_bytes)
                .load(conn)?;

            Ok((
                branch_name,
                title,
                repo_owner,
                repo_name,
                default_branch,
                slack_url,
                base_bytes,
                update_rows,
            ))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    // Reconstruct the Loro doc and export to markdown files.
    let doc = reconstruct_doc(&base_bytes, &update_rows)
        .map_err(|e| ServerFnError::new(format!("reconstruct: {e}")))?;

    let md_files = doc_to_markdown_files(&doc);
    if md_files.is_empty() {
        return Err(ServerFnError::new("No spec files to commit"));
    }

    let files: Vec<FileToCommit> = md_files
        .into_iter()
        .map(|(path, content)| FileToCommit { path, content })
        .collect();

    // Create branch from default branch HEAD.
    let head_sha = github
        .get_branch_head_sha(&repo_owner, &repo_name, &default_branch)
        .await
        .map_err(|e| ServerFnError::new(format!("get HEAD: {e}")))?;

    github
        .create_branch(&repo_owner, &repo_name, &branch_name, &head_sha)
        .await
        .map_err(|e| ServerFnError::new(format!("create branch: {e}")))?;

    // Commit spec files.
    let pr_title = title
        .clone()
        .unwrap_or_else(|| format!("Proposal #{proposal_id}"));
    let commit_msg = format!("spec: {pr_title}");

    github
        .commit_files(
            &repo_owner,
            &repo_name,
            &branch_name,
            &head_sha,
            &commit_msg,
            &files,
        )
        .await
        .map_err(|e| ServerFnError::new(format!("commit files: {e}")))?;

    // Open PR.
    let pr_number = github
        .create_pull_request(
            &repo_owner,
            &repo_name,
            &pr_title,
            &branch_name,
            &default_branch,
            None,
        )
        .await
        .map_err(|e| ServerFnError::new(format!("create PR: {e}")))?;

    // Update proposal status and store PR number.
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    let pr_num_i32 = pr_number as i32;
    conn.interact(move |conn| {
        use crate::db::schema::proposals;
        diesel::update(proposals::table.find(proposal_id))
            .set((
                proposals::status.eq("submitted"),
                proposals::pr_number.eq(Some(pr_num_i32)),
            ))
            .execute(conn)
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))?;

    // r[impl notify.slack]
    if let Some(ref url) = slack_url {
        let text = format!(
            "New spec proposal submitted: *{}* (PR #{pr_number})",
            title
                .as_deref()
                .unwrap_or(&format!("Proposal #{proposal_id}"))
        );
        // Fire-and-forget; don't fail submission on Slack errors.
        let _ = github.send_slack_notification(url, &text).await;
    }

    Ok(())
}

/// Reopen a submitted proposal back to drafting.
// r[impl lifecycle.submitted.reopen]
#[server]
pub async fn reopen_proposal(proposal_id: i32) -> Result<(), ServerFnError> {
    use diesel::prelude::*;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let github = use_context::<crate::github::GitHubClient>()
        .ok_or_else(|| ServerFnError::new("No GitHub client"))?;

    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let (status, pr_number, repo_owner, repo_name) = conn
        .interact(move |conn| {
            use crate::db::schema::{proposals, repositories};

            let (status, pr_number, repo_id): (String, Option<i32>, i32) = proposals::table
                .find(proposal_id)
                .select((
                    proposals::status,
                    proposals::pr_number,
                    proposals::repository_id,
                ))
                .first(conn)?;

            let (repo_owner, repo_name): (String, String) = repositories::table
                .find(repo_id)
                .select((repositories::owner, repositories::name))
                .first(conn)?;

            Ok::<_, diesel::result::Error>((status, pr_number, repo_owner, repo_name))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    if status != "submitted" {
        return Err(ServerFnError::new(format!(
            "cannot reopen a proposal in '{status}' state"
        )));
    }

    // Convert the backing PR to draft and add a comment.
    if let Some(pr_num) = pr_number {
        github
            .convert_pr_to_draft_with_comment(
                &repo_owner,
                &repo_name,
                pr_num as i64,
                "This proposal has been reopened for editing in McBean. The pull request has been converted to draft.",
            )
            .await
            .map_err(|e| ServerFnError::new(format!("convert PR to draft: {e}")))?;
    }

    // Transition back to drafting.
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;
    conn.interact(move |conn| {
        use crate::db::schema::proposals;
        diesel::update(proposals::table.find(proposal_id))
            .set(proposals::status.eq("drafting"))
            .execute(conn)
    })
    .await
    .map_err(|e| ServerFnError::new(format!("{e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("{e}")))?;

    Ok(())
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

    // Bump this to force-refetch the proposal after a state transition.
    let refetch_version = RwSignal::new(0u32);

    let proposal = Resource::new(
        move || (proposal_id(), refetch_version.get()),
        |(pid, _)| get_proposal(pid),
    );
    let blocks_resource = Resource::new(proposal_id, get_proposal_blocks);
    let base_blocks_resource = Resource::new(proposal_id, get_base_blocks);

    let editing_title = RwSignal::new(false);
    let title_draft = RwSignal::new(String::new());

    let save_title_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        let new_title = title_draft.get();
        async move { update_proposal_title(pid, new_title).await }
    });

    // Drafting -> Finalising
    let finalise_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        async move { finalise_proposal(pid).await }
    });

    // Finalising -> Drafting
    let unfinalise_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        async move { unfinalise_proposal(pid).await }
    });

    // Finalising -> Submitted
    let submit_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        async move { submit_proposal(pid).await }
    });

    // Submitted -> Drafting
    let reopen_action = Action::new(move |_: &()| {
        let pid = proposal_id();
        async move { reopen_proposal(pid).await }
    });

    // Refetch proposal after any state transition succeeds.
    Effect::new(move |_| {
        if let Some(Ok(())) = finalise_action.value().get() {
            refetch_version.update(|v| *v = v.wrapping_add(1));
        }
    });
    Effect::new(move |_| {
        if let Some(Ok(())) = unfinalise_action.value().get() {
            refetch_version.update(|v| *v = v.wrapping_add(1));
        }
    });
    Effect::new(move |_| {
        if let Some(Ok(())) = submit_action.value().get() {
            refetch_version.update(|v| *v = v.wrapping_add(1));
        }
    });
    Effect::new(move |_| {
        if let Some(Ok(())) = reopen_action.value().get() {
            refetch_version.update(|v| *v = v.wrapping_add(1));
        }
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
                            let is_finalising = status == "finalising";
                            let is_submitted = status == "submitted";
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
                                                    "finalising" => "is-link",
                                                    "submitted" => "is-primary",
                                                    "in_progress" => "is-info",
                                                    "merged" => "is-success",
                                                    "abandoned" => "is-danger",
                                                    _ => "is-light",
                                                },
                                            )>{status.clone()}</span>
                                        </div>
                                    </div>
                                </div>

                                // ── Action error feedback ─────────────────────
                                {move || {
                                    let errs: Vec<String> = [
                                        finalise_action.value().get().and_then(|r| r.err()),
                                        unfinalise_action.value().get().and_then(|r| r.err()),
                                        submit_action.value().get().and_then(|r| r.err()),
                                        reopen_action.value().get().and_then(|r| r.err()),
                                    ]
                                    .into_iter()
                                    .flatten()
                                    .map(|e| format!("{e}"))
                                    .collect();

                                    if errs.is_empty() {
                                        None
                                    } else {
                                        Some(view! {
                                            <div class="notification is-danger is-light mb-4">
                                                {errs.join("; ")}
                                            </div>
                                        })
                                    }
                                }}

                                // ── Spec content (state-dependent) ────────────
                                // r[impl edit.availability]
                                // r[impl users.collaboration]
                                <Suspense fallback=move || {
                                    view! { <p class="has-text-grey">"Loading spec…"</p> }
                                }>
                                    {move || {
                                        blocks_resource
                                            .get()
                                            .map(|result: Result<Vec<SpecBlock>, _>| match result {
                                                // ── Drafting: full editor ──────────
                                                Ok(_) if is_drafting => {
                                                    let blocks_out = RwSignal::new(Vec::<SpecBlock>::new());
                                                    let sync_error: RwSignal<Option<String>> = RwSignal::new(None);
                                                    // r[impl edit.undo]
                                                    let revert_op: RwSignal<Option<RevertOp>> = RwSignal::new(None);
                                                    let sidebar_title_clone = sidebar_title.clone();
                                                    view! {
                                                        <div style="display: flex; align-items: flex-start; margin: 0 -1.5rem;">
                                                            {move || {
                                                                let blocks = blocks_out.get();
                                                                let (outline, search_entries) = blocks_to_sidebar_data(&blocks, &sidebar_title_clone);
                                                                view! {
                                                                    <SpecSidebar outline=outline search_entries=search_entries />
                                                                }
                                                            }}
                                                            <div style="flex: 1; min-width: 0; padding: 0 1.5rem;">
                                                                <SpecBlockEditor
                                                                    proposal_id=p.id
                                                                    blocks_out=blocks_out
                                                                    sync_error=sync_error
                                                                    revert_op=revert_op
                                                                />
                                                            </div>
                                                            // r[impl proposal.diff.semantic]
                                                            <Suspense fallback=|| view! { <span /> }>
                                                                {move || {
                                                                    base_blocks_resource.get().map(|result| {
                                                                        let initial = result.unwrap_or_default();
                                                                        view! {
                                                                            <ChangelogSidebar
                                                                                initial_blocks=initial
                                                                                blocks=Signal::from(blocks_out)
                                                                                sync_error=sync_error
                                                                                revert_op=revert_op
                                                                            />
                                                                        }
                                                                    })
                                                                }}
                                                            </Suspense>
                                                        </div>
                                                    }
                                                    .into_any()
                                                }
                                                // ── Finalising: review & submit ───
                                                // r[impl lifecycle.finalising]
                                                Ok(blocks) if is_finalising => {
                                                    let base = base_blocks_resource
                                                        .get()
                                                        .and_then(|r| r.ok())
                                                        .unwrap_or_default();
                                                    let submit_err_signal = Signal::derive(move || {
                                                        submit_action
                                                            .value()
                                                            .get()
                                                            .and_then(|r| r.err())
                                                            .map(|e| format!("{e}"))
                                                    });
                                                    let submitting_signal = Signal::derive(move || {
                                                        submit_action.pending().get()
                                                    });
                                                    view! {
                                                        <FinalisingView
                                                            blocks=blocks
                                                            base_blocks=base
                                                            on_back=Callback::new(move |_| {
                                                                unfinalise_action.dispatch(());
                                                            })
                                                            on_submit=Callback::new(move |_| {
                                                                submit_action.dispatch(());
                                                            })
                                                            submitting=submitting_signal
                                                            submit_error=submit_err_signal
                                                        />
                                                    }
                                                    .into_any()
                                                }
                                                // ── Submitted: read-only + reopen ─
                                                // r[impl lifecycle.submitted]
                                                // r[impl lifecycle.submitted.reopen]
                                                Ok(blocks) if is_submitted => {
                                                    let title = sidebar_title.clone();
                                                    let (outline, search_entries) =
                                                        blocks_to_sidebar_data(&blocks, &title);
                                                    view! {
                                                        <div class="notification is-info is-light mb-4">
                                                            <div class="is-flex is-align-items-center is-justify-content-space-between">
                                                                <span>
                                                                    "This proposal has been submitted. A pull request is open in the backing repository."
                                                                </span>
                                                                <button
                                                                    class="button is-small is-warning"
                                                                    disabled=move || reopen_action.pending().get()
                                                                    on:click=move |_| {
                                                                        reopen_action.dispatch(());
                                                                    }
                                                                >
                                                                    {move || {
                                                                        if reopen_action.pending().get() {
                                                                            "Reopening…"
                                                                        } else {
                                                                            "Reopen for Editing"
                                                                        }
                                                                    }}
                                                                </button>
                                                            </div>
                                                        </div>
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
                                                // ── Any other state: read-only ────
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
