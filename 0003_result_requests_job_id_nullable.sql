-- Migration: make job_id nullable in result_requests
--
-- For regular job result requests job_id continues to reference jobs(id).
-- For demo result requests job_id is NULL (no row in the jobs table).
-- The scheduler resolves work_dir from the work_dir column in that case.

ALTER TABLE result_requests
    DROP CONSTRAINT result_requests_job_id_fkey;

ALTER TABLE result_requests
    ALTER COLUMN job_id DROP NOT NULL;

ALTER TABLE result_requests
    ADD CONSTRAINT result_requests_job_id_fkey
        FOREIGN KEY (job_id) REFERENCES jobs(id)
        ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED;
