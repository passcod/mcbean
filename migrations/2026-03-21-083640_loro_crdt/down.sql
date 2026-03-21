ALTER TABLE proposals DROP COLUMN IF EXISTS base_snapshot_id;

DROP TABLE IF EXISTS proposal_loro_updates;
DROP TABLE IF EXISTS spec_snapshots;

CREATE TABLE spec_files (
    id SERIAL PRIMARY KEY,
    spec_id INTEGER NOT NULL REFERENCES specs(id) ON DELETE CASCADE,
    path VARCHAR NOT NULL,
    content TEXT NOT NULL,
    commit_sha VARCHAR NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(spec_id, path)
);

CREATE TABLE proposal_changes (
    id SERIAL PRIMARY KEY,
    proposal_id INTEGER NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
    parent_change_id INTEGER REFERENCES proposal_changes(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    change_type VARCHAR NOT NULL
        CHECK (change_type IN ('user_edit', 'llm_edit', 'undo')),
    llm_prompt TEXT,
    content_snapshot TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
