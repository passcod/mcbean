-- Extend the status CHECK to include the new lifecycle states.
-- Postgres doesn't support ALTER CHECK inline, so drop + re-add.
ALTER TABLE proposals DROP CONSTRAINT proposals_status_check;
ALTER TABLE proposals ADD CONSTRAINT proposals_status_check
    CHECK (status IN ('drafting', 'finalising', 'submitted', 'in_progress', 'merged', 'abandoned'));

-- Track the GitHub PR number so we can update/convert it later.
ALTER TABLE proposals ADD COLUMN pr_number INTEGER;
