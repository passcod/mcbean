use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use super::schema::*;

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i32,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
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
    pub github_url: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub slack_webhook_url: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
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
    pub repository_id: i32,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = specs)]
pub struct NewSpec {
    pub repository_id: i32,
    pub name: String,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = spec_files)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SpecFile {
    pub id: i32,
    pub spec_id: i32,
    pub path: String,
    pub content: String,
    pub commit_sha: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = spec_files)]
pub struct NewSpecFile {
    pub spec_id: i32,
    pub path: String,
    pub content: String,
    pub commit_sha: String,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = proposals)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Proposal {
    pub id: i32,
    pub repository_id: i32,
    pub spec_id: i32,
    pub title: Option<String>,
    pub title_is_user_supplied: bool,
    pub branch_name: String,
    pub status: String,
    pub created_by: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = proposals)]
pub struct NewProposal {
    pub repository_id: i32,
    pub spec_id: i32,
    pub title: Option<String>,
    pub title_is_user_supplied: Option<bool>,
    pub branch_name: String,
    pub status: Option<String>,
    pub created_by: i32,
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = proposal_changes)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProposalChange {
    pub id: i32,
    pub proposal_id: i32,
    pub parent_change_id: Option<i32>,
    pub user_id: i32,
    pub change_type: String,
    pub llm_prompt: Option<String>,
    pub content_snapshot: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = proposal_changes)]
pub struct NewProposalChange {
    pub proposal_id: i32,
    pub parent_change_id: Option<i32>,
    pub user_id: i32,
    pub change_type: String,
    pub llm_prompt: Option<String>,
    pub content_snapshot: String,
}
