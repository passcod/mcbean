-- Drop old raw-content tables (spec_files depends on specs but specs is kept
-- as a name-registry for navigation; drop files first to satisfy FK order).
DROP TABLE IF EXISTS spec_files;

-- Drop old snapshot-based change history.
DROP TABLE IF EXISTS proposal_changes;

-- ---------------------------------------------------------------------------
-- spec_snapshots
-- One row per (repository, commit_sha): the Loro doc bytes that represent the
-- fully-parsed spec tree at that point in git history.  Multiple proposals may
-- share the same base snapshot when they were branched from the same commit.
-- ---------------------------------------------------------------------------
CREATE TABLE spec_snapshots (
    id              SERIAL PRIMARY KEY,
    repository_id   INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_sha      VARCHAR NOT NULL,
    -- Loro doc exported as ExportMode::Snapshot bytes.
    loro_bytes      BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (repository_id, commit_sha)
);

-- ---------------------------------------------------------------------------
-- proposals: record which snapshot this proposal forked from.
-- Nullable because a proposal can be created before the snapshot row exists
-- (snapshot is written by the sync task; proposal by the user).  The server
-- resolves the snapshot lazily on first access and fills it in.
-- ---------------------------------------------------------------------------
ALTER TABLE proposals
    ADD COLUMN base_snapshot_id INTEGER REFERENCES spec_snapshots(id);

-- ---------------------------------------------------------------------------
-- proposal_loro_updates
-- Each row is a Loro delta-update (ExportMode::Updates) produced by one peer
-- session.  Replaying all rows in id order over the base snapshot reconstructs
-- the current state of the proposal.
--
-- peer_id is the Loro PeerID (u64) stored as BIGINT via bitcast; arithmetic
-- on this column is meaningless, it is an opaque identifier only.
-- ---------------------------------------------------------------------------
CREATE TABLE proposal_loro_updates (
    id              SERIAL PRIMARY KEY,
    proposal_id     INTEGER NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id),
    -- Loro PeerID (u64) bitcast to i64 for storage.
    peer_id         BIGINT NOT NULL,
    update_bytes    BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX proposal_loro_updates_proposal_id_idx
    ON proposal_loro_updates (proposal_id, id);
