-- Migration: make job_id nullable in result_requests
--
-- Demo result requests have no row in the jobs table, so job_id must be
-- optional. The scheduler uses work_dir directly for those rows and skips
-- the JOIN with the jobs table entirely.
--
-- Regular job result requests continue to populate job_id as before.

ALTER TABLE result_requests
    ALTER COLUMN job_id DROP NOT NULL;
