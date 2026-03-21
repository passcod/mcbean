use std::time::Duration;

use diesel::prelude::*;
use tracing::{error, info, warn};

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

    // Fetch and parse tracey config
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

    // Resolve include patterns and fetch file contents for each spec
    let mut spec_files_by_spec: Vec<(String, Vec<(String, String)>)> = Vec::new();

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
        spec_files_by_spec.push((spec_def.name.clone(), files));
    }

    // Update database in a transaction
    let conn = pool.get().await?;
    let sha = head_sha.clone();

    conn.interact(move |conn| {
        use crate::db::schema::{repositories, spec_files, specs};

        conn.transaction::<(), diesel::result::Error, _>(|conn| {
            // Upsert each spec and its files
            for (spec_name, files) in &spec_files_by_spec {
                // Find or create the spec
                let existing: Option<i32> = specs::table
                    .filter(specs::repository_id.eq(repo_id))
                    .filter(specs::name.eq(spec_name))
                    .select(specs::id)
                    .first(conn)
                    .optional()?;

                let spec_id = match existing {
                    Some(sid) => {
                        diesel::update(specs::table.filter(specs::id.eq(sid)))
                            .set(specs::updated_at.eq(diesel::dsl::now))
                            .execute(conn)?;
                        sid
                    }
                    None => {
                        let (sid,): (i32,) = diesel::insert_into(specs::table)
                            .values((specs::repository_id.eq(repo_id), specs::name.eq(spec_name)))
                            .returning((specs::id,))
                            .get_result(conn)?;
                        sid
                    }
                };

                // Delete old files and insert fresh ones
                diesel::delete(spec_files::table.filter(spec_files::spec_id.eq(spec_id)))
                    .execute(conn)?;

                for (file_path, file_content) in files {
                    diesel::insert_into(spec_files::table)
                        .values((
                            spec_files::spec_id.eq(spec_id),
                            spec_files::path.eq(file_path),
                            spec_files::content.eq(file_content),
                            spec_files::commit_sha.eq(&sha),
                        ))
                        .execute(conn)?;
                }
            }

            // Remove specs that no longer exist in the config
            let spec_names: Vec<&String> = spec_files_by_spec.iter().map(|(n, _)| n).collect();
            diesel::delete(
                specs::table
                    .filter(specs::repository_id.eq(repo_id))
                    .filter(specs::name.ne_all(&spec_names)),
            )
            .execute(conn)?;

            // Update the synced SHA
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
