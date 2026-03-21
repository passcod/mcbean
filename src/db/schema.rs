// @generated automatically by Diesel CLI.

diesel::table! {
    proposal_changes (id) {
        id -> Int4,
        proposal_id -> Int4,
        parent_change_id -> Nullable<Int4>,
        user_id -> Int4,
        change_type -> Varchar,
        llm_prompt -> Nullable<Text>,
        content_snapshot -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    proposals (id) {
        id -> Int4,
        repository_id -> Int4,
        title -> Nullable<Varchar>,
        title_is_user_supplied -> Bool,
        branch_name -> Varchar,
        status -> Varchar,
        created_by -> Int4,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    repositories (id) {
        id -> Int4,
        github_url -> Varchar,
        owner -> Varchar,
        name -> Varchar,
        default_branch -> Varchar,
        slack_webhook_url -> Nullable<Varchar>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        last_synced_sha -> Nullable<Varchar>,
    }
}

diesel::table! {
    spec_files (id) {
        id -> Int4,
        spec_id -> Int4,
        path -> Varchar,
        content -> Text,
        commit_sha -> Varchar,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    specs (id) {
        id -> Int4,
        repository_id -> Int4,
        name -> Varchar,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        email -> Varchar,
        display_name -> Nullable<Varchar>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::joinable!(proposal_changes -> proposals (proposal_id));
diesel::joinable!(proposal_changes -> users (user_id));
diesel::joinable!(proposals -> repositories (repository_id));
diesel::joinable!(proposals -> users (created_by));
diesel::joinable!(spec_files -> specs (spec_id));
diesel::joinable!(specs -> repositories (repository_id));

diesel::allow_tables_to_appear_in_same_query!(
    proposal_changes,
    proposals,
    repositories,
    spec_files,
    specs,
    users,
);
