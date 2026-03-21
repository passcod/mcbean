diesel::table! {
    users (id) {
        id -> Int4,
        // r[impl users.identity]
        email -> Varchar,
        display_name -> Nullable<Varchar>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    repositories (id) {
        id -> Int4,
        // r[impl repo.connect]
        github_url -> Varchar,
        owner -> Varchar,
        name -> Varchar,
        default_branch -> Varchar,
        // r[impl notify.slack]
        slack_webhook_url -> Nullable<Varchar>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    specs (id) {
        id -> Int4,
        // r[impl repo.multi-spec]
        repository_id -> Int4,
        name -> Varchar,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    spec_files (id) {
        id -> Int4,
        // r[impl repo.multi-file]
        spec_id -> Int4,
        path -> Varchar,
        content -> Text,
        commit_sha -> Varchar,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    proposals (id) {
        id -> Int4,
        repository_id -> Int4,
        spec_id -> Int4,
        title -> Nullable<Varchar>,
        // r[impl proposal.title.user-priority]
        title_is_user_supplied -> Bool,
        // r[impl proposal.git.backing]
        branch_name -> Varchar,
        // r[impl lifecycle.drafting]
        status -> Varchar,
        created_by -> Int4,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    proposal_changes (id) {
        id -> Int4,
        proposal_id -> Int4,
        // r[impl edit.history]
        parent_change_id -> Nullable<Int4>,
        // r[impl users.identity]
        user_id -> Int4,
        change_type -> Varchar,
        llm_prompt -> Nullable<Text>,
        // r[impl edit.history]
        content_snapshot -> Text,
        created_at -> Timestamptz,
    }
}

diesel::joinable!(specs -> repositories (repository_id));
diesel::joinable!(spec_files -> specs (spec_id));
diesel::joinable!(proposals -> repositories (repository_id));
diesel::joinable!(proposals -> specs (spec_id));
diesel::joinable!(proposals -> users (created_by));
diesel::joinable!(proposal_changes -> proposals (proposal_id));
diesel::joinable!(proposal_changes -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    users,
    repositories,
    specs,
    spec_files,
    proposals,
    proposal_changes,
);
