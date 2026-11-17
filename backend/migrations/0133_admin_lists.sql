-- The instance admin's lists on /api/v2 (docs/api-v2-completion-plan.md, phase 12) and the
-- disc-number back-fill as a job.
--
-- `GET /admin/jobs` pages the jobs newest first, all of them or by kind or state. A checksum job
-- is made per stored file, so the table can hold one row per track of a big library; each page
-- must come from an index, not a sort of the whole table.
CREATE INDEX jobs_created_idx ON jobs (created_at DESC, id DESC);
CREATE INDEX jobs_type_created_idx ON jobs (job_type, created_at DESC, id DESC);
CREATE INDEX jobs_status_created_idx ON jobs (status, created_at DESC, id DESC);

-- The tracks whose file was never read for its disc number: only uploads made before 0083, so
-- the index is small, and empty once the back-fill has run.
CREATE INDEX music_tracks_disc_unchecked_idx ON music_tracks_all (created_at, id) WHERE NOT disc_checked;
