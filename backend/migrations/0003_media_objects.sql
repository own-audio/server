-- Migration 0003: media objects
-- ─────────────────────────────────────────────────
-- A media_object is a private binary stored in the object store (MinIO/S3).
-- All episode audio, audiobook files, and cover images are referenced here.
-- Rows store only metadata; the actual bytes live in the bucket.

CREATE TABLE media_objects (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    bucket       TEXT        NOT NULL,
    object_key   TEXT        NOT NULL,
    content_type TEXT        NOT NULL,
    size_bytes   BIGINT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT media_objects_bucket_key_uq UNIQUE (bucket, object_key)
);
