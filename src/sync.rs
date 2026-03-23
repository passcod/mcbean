use std::time::Duration;

use diesel::prelude::*;
use snafu::{FromString, ResultExt, Whatever};
use tracing::{error, info, warn};

use crate::components::loro_doc::build_doc_from_specs;
use crate::db::DbPool;
use crate::github::{GitHubClient, PullRequestResponse};

const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Spawn the background spec-sync task. Call once at server startup.
pub fn spawn(pool: DbPool, github: GitHubClient) {
    tokio::spawn(async move {
        info!(
            "spec sync task started (interval: {}s)",
            SYNC_INTERVAL.as_secs()
        );
        loop {
            tokio::time::sleep(SYNC_INTERVAL).await;
            if let Err(e) = sync_all(&pool, &github).await {
                error!(error = %e, "spec sync cycle failed");
            }
            if let Err(e) = sync_proposals(&pool, &github).await {
                error!(error = %e, "proposal sync cycle failed");
            }
        }
    });
}

async fn sync_all(pool: &DbPool, github: &GitHubClient) -> Result<(), Whatever> {
    info!("spec sync: starting cycle");

    let conn = pool.get().await.whatever_context("pool error")?;

    let repos: Vec<(i32, String, String, String, Option<String>)> = conn
        .interact(|conn| {
            use crate::db::schema::repositories::dsl::*;
            repositories
                .select((id, owner, name, default_branch, last_synced_sha))
                .load(conn)
        })
        .await
        .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
        .whatever_context("query error")?;

    info!(repo_count = repos.len(), "spec sync: loaded repositories");

    for (repo_id, repo_owner, repo_name, branch, old_sha) in repos {
        if let Err(e) = sync_repo(
            pool,
            github,
            repo_id,
            &repo_owner,
            &repo_name,
            &branch,
            old_sha.as_deref(),
        )
        .await
        {
            error!(
                repo_id,
                owner = %repo_owner,
                name = %repo_name,
                error = %e,
                "spec sync: failed to sync repo"
            );
        }
    }

    info!("spec sync: cycle complete");
    Ok(())
}

// ── Proposal lifecycle sync ───────────────────────────────────────────────────

/// Poll GitHub for PR state changes on submitted/in-progress proposals and
/// update their lifecycle accordingly.
async fn sync_proposals(pool: &DbPool, github: &GitHubClient) -> Result<(), Whatever> {
    info!("proposal sync: starting cycle");

    let conn = pool.get().await.whatever_context("pool error")?;

    // Load proposals that need monitoring: submitted or in_progress.
    let proposals: Vec<(i32, String, Option<i32>, i32, String)> = conn
        .interact(|conn| {
            use crate::db::schema::proposals::dsl::*;
            proposals
                .filter(status.eq("submitted").or(status.eq("in_progress")))
                .select((id, branch_name, pr_number, repository_id, status))
                .load(conn)
        })
        .await
        .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
        .whatever_context("query error")?;

    if proposals.is_empty() {
        info!("proposal sync: no active proposals to check");
        return Ok(());
    }

    info!(
        count = proposals.len(),
        "proposal sync: checking active proposals"
    );

    for (prop_id, branch, pr_num, repo_id, current_status) in proposals {
        if let Err(e) = sync_single_proposal(
            pool,
            github,
            prop_id,
            &branch,
            pr_num,
            repo_id,
            &current_status,
        )
        .await
        {
            error!(
                proposal_id = prop_id,
                error = %e,
                "proposal sync: failed to sync proposal"
            );
        }
    }

    info!("proposal sync: cycle complete");
    Ok(())
}

async fn sync_single_proposal(
    pool: &DbPool,
    github: &GitHubClient,
    proposal_id: i32,
    branch_name: &str,
    pr_number: Option<i32>,
    repo_id: i32,
    current_status: &str,
) -> Result<(), Whatever> {
    let conn = pool.get().await.whatever_context("pool error")?;

    // Load repo owner/name.
    let (repo_owner, repo_name): (String, String) = conn
        .interact(move |conn| {
            use crate::db::schema::repositories;
            repositories::table
                .find(repo_id)
                .select((repositories::owner, repositories::name))
                .first(conn)
        })
        .await
        .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
        .whatever_context("query error")?;

    // Check the backing PR state (if we have one).
    if let Some(pr_num) = pr_number {
        let pr: PullRequestResponse = github
            .get_pull_request(&repo_owner, &repo_name, pr_num as i64)
            .await
            .whatever_context("get pull request")?;

        // r[impl lifecycle.merged]
        if pr.merged == Some(true) {
            info!(proposal_id, pr_num, "proposal sync: PR merged");
            set_proposal_status(pool, proposal_id, "merged").await?;
            return Ok(());
        }

        // r[impl lifecycle.abandoned]
        if pr.state == "closed" {
            info!(
                proposal_id,
                pr_num, "proposal sync: PR closed without merge"
            );
            set_proposal_status(pool, proposal_id, "drafting").await?;
            return Ok(());
        }
    }

    // r[impl lifecycle.in-progress.trigger]
    // Check for implementation PRs targeting the proposal branch.
    let impl_prs: Vec<PullRequestResponse> = github
        .list_prs_with_base(&repo_owner, &repo_name, branch_name)
        .await
        .whatever_context("list PRs with base")?;

    let has_impl_prs = !impl_prs.is_empty();

    match (current_status, has_impl_prs) {
        // r[impl lifecycle.in-progress.trigger]
        ("submitted", true) => {
            info!(
                proposal_id,
                impl_pr_count = impl_prs.len(),
                "proposal sync: implementation PRs detected, transitioning to in_progress"
            );
            // r[impl lifecycle.in-progress.frozen]
            set_proposal_status(pool, proposal_id, "in_progress").await?;
        }
        // r[impl lifecycle.in-progress.frozen]
        ("in_progress", false) => {
            // All implementation PRs resolved; back to submitted.
            info!(
                proposal_id,
                "proposal sync: all implementation PRs resolved, returning to submitted"
            );
            set_proposal_status(pool, proposal_id, "submitted").await?;
        }
        _ => {
            // No state change needed.
        }
    }

    Ok(())
}

async fn set_proposal_status(
    pool: &DbPool,
    proposal_id: i32,
    new_status: &str,
) -> Result<(), Whatever> {
    let conn = pool.get().await.whatever_context("pool error")?;
    let status = new_status.to_owned();
    conn.interact(move |conn| {
        use crate::db::schema::proposals;
        diesel::update(proposals::table.find(proposal_id))
            .set(proposals::status.eq(&status))
            .execute(conn)
    })
    .await
    .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
    .whatever_context("update error")?;
    Ok(())
}

async fn sync_repo(
    pool: &DbPool,
    github: &GitHubClient,
    repo_id: i32,
    owner: &str,
    name: &str,
    branch: &str,
    old_sha: Option<&str>,
) -> Result<(), Whatever> {
    let head_sha = github
        .get_branch_head_sha(owner, name, branch)
        .await
        .whatever_context("get branch head SHA")?;

    if old_sha == Some(head_sha.as_str()) {
        info!(repo_id, %head_sha, "spec sync: repo unchanged, skipping");
        return Ok(());
    }

    info!(
        repo_id,
        old_sha = old_sha.unwrap_or("none"),
        %head_sha,
        "spec sync: repo has new commits, re-fetching specs"
    );

    // Check whether we already have a snapshot for this exact commit SHA.
    // This can happen if the sync task runs twice before the repo advances.
    {
        let conn = pool.get().await.whatever_context("pool error")?;
        let sha = head_sha.clone();
        let already_stored: bool = conn
            .interact(move |conn| {
                use crate::db::schema::spec_snapshots::dsl::*;
                let count: i64 = spec_snapshots
                    .filter(repository_id.eq(repo_id))
                    .filter(commit_sha.eq(&sha))
                    .count()
                    .get_result(conn)?;
                Ok::<_, diesel::result::Error>(count > 0)
            })
            .await
            .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
            .whatever_context("query error")?;

        if already_stored {
            info!(repo_id, %head_sha, "spec sync: snapshot already stored for this SHA");
            // Still update last_synced_sha so we skip next time.
            let conn = pool.get().await.whatever_context("pool error")?;
            let sha = head_sha.clone();
            conn.interact(move |conn| {
                use crate::db::schema::repositories::dsl::*;
                diesel::update(repositories.filter(id.eq(repo_id)))
                    .set(last_synced_sha.eq(&sha))
                    .execute(conn)
            })
            .await
            .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
            .whatever_context("update error")?;
            return Ok(());
        }
    }

    // Fetch and parse tracey config.
    let config_content = match github
        .get_file_contents(owner, name, ".config/tracey/config.styx", branch)
        .await
    {
        Ok(f) => f.content,
        Err(crate::github::GitHubError::NotFound { .. }) => {
            warn!(
                repo_id,
                "spec sync: tracey config disappeared from repo, skipping"
            );
            return Ok(());
        }
        Err(e) => return Err(e).whatever_context("fetch tracey config"),
    };

    let config: tracey_config::Config =
        facet_styx::from_str(&config_content).whatever_context("parse tracey config")?;

    if config.specs.is_empty() {
        warn!(repo_id, "spec sync: tracey config has no specs, skipping");
        return Ok(());
    }

    // Resolve include patterns and fetch file contents for each spec.
    // Build a list of (spec_name, [(file_path, file_content)]) for the Loro builder.
    let mut specs_with_files: Vec<(String, Vec<(String, String)>)> = Vec::new();

    for spec_def in &config.specs {
        let fetched = github
            .resolve_include_patterns(owner, name, branch, &spec_def.include)
            .await
            .whatever_context("resolve include patterns")?;

        info!(
            repo_id,
            spec = %spec_def.name,
            file_count = fetched.len(),
            "spec sync: resolved spec files"
        );

        let files: Vec<(String, String)> =
            fetched.into_iter().map(|f| (f.path, f.content)).collect();

        specs_with_files.push((spec_def.name.clone(), files));
    }

    // Build the Loro doc from all specs and export as a snapshot.
    let doc = build_doc_from_specs(&specs_with_files).await;
    let loro_bytes = doc
        .export(loro::ExportMode::Snapshot)
        .whatever_context("loro export")?;

    info!(
        repo_id,
        %head_sha,
        snapshot_bytes = loro_bytes.len(),
        "spec sync: built Loro snapshot"
    );

    // Persist everything in a single transaction.
    let conn = pool.get().await.whatever_context("pool error")?;
    let sha = head_sha.clone();
    let spec_names: Vec<String> = specs_with_files.iter().map(|(n, _)| n.clone()).collect();

    conn.interact(move |conn| {
        use crate::db::schema::{repositories, spec_snapshots, specs};

        conn.transaction::<(), diesel::result::Error, _>(|conn| {
            // Upsert spec name rows (kept for navigation queries).
            for spec_name in &spec_names {
                let existing: Option<i32> = specs::table
                    .filter(specs::repository_id.eq(repo_id))
                    .filter(specs::name.eq(spec_name))
                    .select(specs::id)
                    .first(conn)
                    .optional()?;

                if existing.is_none() {
                    diesel::insert_into(specs::table)
                        .values((specs::repository_id.eq(repo_id), specs::name.eq(spec_name)))
                        .execute(conn)?;
                } else {
                    diesel::update(
                        specs::table
                            .filter(specs::repository_id.eq(repo_id))
                            .filter(specs::name.eq(spec_name)),
                    )
                    .set(specs::updated_at.eq(diesel::dsl::now))
                    .execute(conn)?;
                }
            }

            // Remove specs that no longer exist in the config.
            diesel::delete(
                specs::table
                    .filter(specs::repository_id.eq(repo_id))
                    .filter(specs::name.ne_all(&spec_names)),
            )
            .execute(conn)?;

            // Insert the new snapshot (unique constraint on repository_id + commit_sha
            // prevents duplicates if two sync cycles race).
            diesel::insert_into(spec_snapshots::table)
                .values((
                    spec_snapshots::repository_id.eq(repo_id),
                    spec_snapshots::commit_sha.eq(&sha),
                    spec_snapshots::loro_bytes.eq(&loro_bytes),
                ))
                .on_conflict((spec_snapshots::repository_id, spec_snapshots::commit_sha))
                .do_nothing()
                .execute(conn)?;

            // Advance the synced SHA.
            diesel::update(repositories::table.filter(repositories::id.eq(repo_id)))
                .set(repositories::last_synced_sha.eq(&sha))
                .execute(conn)?;

            Ok(())
        })
    })
    .await
    .map_err(|e| snafu::Whatever::without_source(format!("interact error: {e}")))?
    .whatever_context("transaction error")?;

    info!(repo_id, %head_sha, "spec sync: repo updated successfully");
    Ok(())
}
