-- Migration: add optional work_dir to result_requests
--
-- For normal jobs this column stays NULL and the scheduler resolves
-- work_dir via JOIN with the jobs table (existing behaviour).
-- For demo result requests it is populated by the API server with the
-- demo's absolute path (e.g. /sisar/demos/<name>), so the scheduler
-- never needs to touch the jobs table for those rows.

ALTER TABLE result_requests
    ADD COLUMN IF NOT EXISTS work_dir TEXT NULL;
