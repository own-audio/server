-- Migration 0039: content checksums on media_objects
-- ─────────────────────────────────────────────────
-- DEDUPLICATION_PLAN.md (audio2-mac) P2 — the evidence a byte-identical-file
-- duplicate check is built on. Lives on media_objects, not music_tracks: the
-- object row is where size_bytes/content_type already live, and audiobook
-- files reference this same table, so a later audiobook dedup phase gets
-- this column for free.
--
-- Nullable, computed asynchronously by a "media_checksum" job (see
-- backend/src/jobs/worker.rs) enqueued after each upload and swept for any
-- object that doesn't have one yet — nothing here blocks the upload response
-- on hashing a large file.

ALTER TABLE media_objects ADD COLUMN sha256 TEXT;

-- Partial: only rows that actually have a hash are worth indexing, and most
-- objects (cover images, everything uploaded before this migration) start
-- out NULL.
CREATE INDEX media_objects_sha256_idx ON media_objects (sha256) WHERE sha256 IS NOT NULL;
