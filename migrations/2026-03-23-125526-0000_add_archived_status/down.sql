ALTER TABLE proposals DROP CONSTRAINT proposals_status_check;
ALTER TABLE proposals ADD CONSTRAINT proposals_status_check
    CHECK (status IN ('drafting', 'finalising', 'submitted', 'in_progress', 'merged', 'abandoned'));
