use std::time::Duration;

use diesel::prelude::*;
use tracing::{error, info, warn};

use crate::components::loro_doc::build_doc_from_specs;
use crate::db::DbPool;
use crate::github::GitHubClient;

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
        }
    });
}

async fn sync_all(pool: &DbPool, github: &GitHubClient) -> anyhow::Result<()> {
    info!("spec sync: starting cycle");

    let conn = pool.get().await?;

    let repos: Vec<(i32, String, String, String, Option<String>)> = conn
        .interact(|conn| {
            use crate::db::schema::repositories::dsl::*;
            repositories
                .select((id, owner, name, default_branch, last_synced_sha))
                .load(conn)
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))?
        .map_err(|e| anyhow::anyhow!("query error: {e}"))?;

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

async fn sync_repo(
    pool: &DbPool,
    github: &GitHubClient,
    repo_id: i32,
    owner: &str,
    name: &str,
    branch: &str,
    old_sha: Option<&str>,
) -> anyhow::Result<()> {
    let head_sha = github.get_branch_head_sha(owner, name, branch).await?;

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
        let conn = pool.get().await?;
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
            .map_err(|e| anyhow::anyhow!("interact error: {e}"))?
            .map_err(|e: diesel::result::Error| anyhow::anyhow!("query error: {e}"))?;

        if already_stored {
            info!(repo_id, %head_sha, "spec sync: snapshot already stored for this SHA");
            // Still update last_synced_sha so we skip next time.
            let conn = pool.get().await?;
            let sha = head_sha.clone();
            conn.interact(move |conn| {
                use crate::db::schema::repositories::dsl::*;
                diesel::update(repositories.filter(id.eq(repo_id)))
                    .set(last_synced_sha.eq(&sha))
                    .execute(conn)
            })
            .await
            .map_err(|e| anyhow::anyhow!("interact error: {e}"))?
            .map_err(|e: diesel::result::Error| anyhow::anyhow!("update error: {e}"))?;
            return Ok(());
        }
    }

    // Fetch and parse tracey config.
    let config_content = match github
        .get_file_contents(owner, name, ".config/tracey/config.styx", branch)
        .await
    {
        Ok(f) => f.content,
        Err(crate::github::GitHubError::NotFound(_)) => {
            warn!(
                repo_id,
                "spec sync: tracey config disappeared from repo, skipping"
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let config: tracey_config::Config = facet_styx::from_str(&config_content)
        .map_err(|e| anyhow::anyhow!("failed to parse tracey config: {e}"))?;

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
            .await?;

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
        .map_err(|e| anyhow::anyhow!("loro export failed: {e}"))?;

    info!(
        repo_id,
        %head_sha,
        snapshot_bytes = loro_bytes.len(),
        "spec sync: built Loro snapshot"
    );

    // Persist everything in a single transaction.
    let conn = pool.get().await?;
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
    .map_err(|e| anyhow::anyhow!("interact error: {e}"))?
    .map_err(|e: diesel::result::Error| anyhow::anyhow!("transaction error: {e}"))?;

    info!(repo_id, %head_sha, "spec sync: repo updated successfully");
    Ok(())
}
