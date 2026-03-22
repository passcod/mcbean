use diesel::prelude::*;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::schema::*;

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i32,
    // r[impl users.identity]
    pub email: String,
    pub display_name: Option<String>,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub updated_at: Timestamp,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = repositories)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Repository {
    pub id: i32,
    // r[impl repo.connect]
    pub github_url: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    // r[impl notify.slack]
    pub slack_webhook_url: Option<String>,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub updated_at: Timestamp,
    pub last_synced_sha: Option<String>,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = repositories)]
pub struct NewRepository {
    pub github_url: String,
    pub owner: String,
    pub name: String,
    pub default_branch: Option<String>,
    pub slack_webhook_url: Option<String>,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = specs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Spec {
    pub id: i32,
    // r[impl repo.multi-spec]
    pub repository_id: i32,
    pub name: String,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub updated_at: Timestamp,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = specs)]
pub struct NewSpec {
    pub repository_id: i32,
    pub name: String,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = spec_snapshots)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SpecSnapshot {
    pub id: i32,
    pub repository_id: i32,
    pub commit_sha: String,
    pub loro_bytes: Vec<u8>,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = spec_snapshots)]
pub struct NewSpecSnapshot {
    pub repository_id: i32,
    pub commit_sha: String,
    pub loro_bytes: Vec<u8>,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = proposals)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Proposal {
    pub id: i32,
    pub repository_id: i32,
    pub title: Option<String>,
    // r[impl proposal.title.user-priority]
    pub title_is_user_supplied: bool,
    // r[impl proposal.git.backing]
    pub branch_name: String,
    // r[impl lifecycle.drafting]
    pub status: String,
    pub created_by: i32,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub updated_at: Timestamp,
    pub base_snapshot_id: Option<i32>,
    pub pr_number: Option<i32>,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = proposals)]
pub struct NewProposal {
    pub repository_id: i32,
    pub title: Option<String>,
    pub title_is_user_supplied: Option<bool>,
    pub branch_name: String,
    pub status: Option<String>,
    pub created_by: i32,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = proposal_loro_updates)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProposalLoroUpdate {
    pub id: i32,
    pub proposal_id: i32,
    // r[impl edit.history]
    pub user_id: i32,
    // Loro PeerID (u64) bitcast to i64 for BIGINT storage.
    pub peer_id: i64,
    pub update_bytes: Vec<u8>,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = proposal_loro_updates)]
pub struct NewProposalLoroUpdate {
    pub proposal_id: i32,
    pub user_id: i32,
    pub peer_id: i64,
    pub update_bytes: Vec<u8>,
}
