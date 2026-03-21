use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql, FromSqlRow};
use diesel::expression::AsExpression;
use diesel::prelude::*;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Varchar;
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
#[diesel(table_name = spec_files)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SpecFile {
    pub id: i32,
    // r[impl repo.multi-file]
    pub spec_id: i32,
    pub path: String,
    pub content: String,
    pub commit_sha: String,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub updated_at: Timestamp,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsExpression, FromSqlRow)]
#[diesel(sql_type = Varchar)]
pub enum ChangeType {
    UserEdit,
    LlmEdit,
    Undo,
}

impl<DB: Backend> ToSql<Varchar, DB> for ChangeType
where
    str: ToSql<Varchar, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> serialize::Result {
        let s = match self {
            ChangeType::UserEdit => "user_edit",
            ChangeType::LlmEdit => "llm_edit",
            ChangeType::Undo => "undo",
        };
        s.to_sql(out)
    }
}

impl<DB: Backend> FromSql<Varchar, DB> for ChangeType
where
    String: FromSql<Varchar, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        match String::from_sql(bytes)?.as_str() {
            "user_edit" => Ok(ChangeType::UserEdit),
            "llm_edit" => Ok(ChangeType::LlmEdit),
            "undo" => Ok(ChangeType::Undo),
            other => Err(format!("unknown change_type: {other}").into()),
        }
    }
}

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = proposal_changes)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProposalChange {
    pub id: i32,
    pub proposal_id: i32,
    // r[impl edit.history]
    pub parent_change_id: Option<i32>,
    pub user_id: i32,
    pub change_type: ChangeType,
    pub llm_prompt: Option<String>,
    // r[impl edit.history]
    pub content_snapshot: String,
    #[diesel(deserialize_as = jiff_diesel::Timestamp, serialize_as = jiff_diesel::Timestamp)]
    pub created_at: Timestamp,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = proposal_changes)]
pub struct NewProposalChange {
    pub proposal_id: i32,
    pub parent_change_id: Option<i32>,
    pub user_id: i32,
    pub change_type: ChangeType,
    pub llm_prompt: Option<String>,
    pub content_snapshot: String,
}
