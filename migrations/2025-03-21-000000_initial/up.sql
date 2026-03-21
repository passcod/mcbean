CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email VARCHAR NOT NULL UNIQUE, -- r[impl users.identity]
    display_name VARCHAR,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE repositories (
    id SERIAL PRIMARY KEY,
    github_url VARCHAR NOT NULL UNIQUE, -- r[impl repo.connect]
    owner VARCHAR NOT NULL,
    name VARCHAR NOT NULL,
    default_branch VARCHAR NOT NULL DEFAULT 'main',
    slack_webhook_url VARCHAR, -- r[impl notify.slack]
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE specs (
    id SERIAL PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE, -- r[impl repo.multi-spec]
    name VARCHAR NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(repository_id, name)
);

CREATE TABLE spec_files (
    id SERIAL PRIMARY KEY,
    spec_id INTEGER NOT NULL REFERENCES specs(id) ON DELETE CASCADE, -- r[impl repo.multi-file]
    path VARCHAR NOT NULL,
    content TEXT NOT NULL,
    commit_sha VARCHAR NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(spec_id, path)
);

CREATE TABLE proposals (
    id SERIAL PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    spec_id INTEGER NOT NULL REFERENCES specs(id) ON DELETE CASCADE,
    title VARCHAR,
    title_is_user_supplied BOOLEAN NOT NULL DEFAULT FALSE, -- r[impl proposal.title.user-priority]
    branch_name VARCHAR NOT NULL UNIQUE, -- r[impl proposal.git.backing]
    status VARCHAR NOT NULL DEFAULT 'drafting' -- r[impl lifecycle.drafting]
        CHECK (status IN ('drafting', 'in_progress', 'merged', 'abandoned')),
    created_by INTEGER NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE proposal_changes (
    id SERIAL PRIMARY KEY,
    proposal_id INTEGER NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
    parent_change_id INTEGER REFERENCES proposal_changes(id), -- r[impl edit.history]
    user_id INTEGER NOT NULL REFERENCES users(id),
    change_type VARCHAR NOT NULL
        CHECK (change_type IN ('user_edit', 'llm_edit', 'undo')),
    llm_prompt TEXT,
    content_snapshot TEXT NOT NULL, -- r[impl edit.history]
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
