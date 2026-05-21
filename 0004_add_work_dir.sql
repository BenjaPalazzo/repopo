-- Add nullable work_dir to result_requests.
-- Used for demos: allows scheduler to bypass jobs table.

ALTER TABLE result_requests
    ADD COLUMN work_dir TEXT NULL;
