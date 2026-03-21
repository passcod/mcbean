// @generated automatically by Diesel CLI.

diesel::table! {
    proposal_loro_updates (id) {
        id -> Int4,
        proposal_id -> Int4,
        user_id -> Int4,
        peer_id -> Int8,
        update_bytes -> Bytea,
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
        base_snapshot_id -> Nullable<Int4>,
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
    spec_snapshots (id) {
        id -> Int4,
        repository_id -> Int4,
        commit_sha -> Varchar,
        loro_bytes -> Bytea,
        created_at -> Timestamptz,
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

diesel::joinable!(proposal_loro_updates -> proposals (proposal_id));
diesel::joinable!(proposal_loro_updates -> users (user_id));
diesel::joinable!(proposals -> repositories (repository_id));
diesel::joinable!(proposals -> spec_snapshots (base_snapshot_id));
diesel::joinable!(proposals -> users (created_by));
diesel::joinable!(spec_snapshots -> repositories (repository_id));
diesel::joinable!(specs -> repositories (repository_id));

diesel::allow_tables_to_appear_in_same_query!(
    proposal_loro_updates,
    proposals,
    repositories,
    spec_snapshots,
    specs,
    users,
);
