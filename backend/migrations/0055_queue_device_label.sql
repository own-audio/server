-- Migration 0055: optional human-readable device label for play-queue sync
-- ─────────────────────────────────────────────────────────────────────────
-- `updated_by_device` stays the coarse platform string ("ios"/"android"/...).
-- This carries an optional per-device name (e.g. "Kornel's Pixel") so two
-- devices of the same platform on one account can be told apart in the
-- cross-device follow prompt, without overloading `updated_by_device`.
ALTER TABLE play_queues ADD COLUMN updated_by_device_label TEXT;
