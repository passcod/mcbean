ALTER TABLE proposals DROP COLUMN pr_number;

ALTER TABLE proposals DROP CONSTRAINT proposals_status_check;
ALTER TABLE proposals ADD CONSTRAINT proposals_status_check
    CHECK (status IN ('drafting', 'in_progress', 'merged', 'abandoned'));
