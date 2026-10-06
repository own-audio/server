-- Migration 0073: the opt-in that starts library measurement
-- ──────────────────────────────────────────────────────────
-- docs/music-signals-and-smart-playlists-plan.md §3.4.
--
-- Measuring a library means fetching every object back out of R2 and decoding
-- it. Egress is free and the operations cost fractions of a cent, so this is
-- not a money question — it is real work on real hardware, and it happens
-- because someone asked for it, not by default.
--
-- WHY IT IS A FAMILY SETTING AND NOT A PERSONAL ONE
--
-- The data is a property of the recording, not of a listener: tempo and
-- loudness are the same for everyone. So one member enabling it measures the
-- shared library once, for all of them. A per-user checkbox would imply
-- per-user results, which is not what happens — taste is per user (see 0070),
-- but measurement is not.
--
-- WHAT THE UI MUST NOT SAY
--
-- This is not a data-processing consent. Nothing leaves the server, nothing is
-- shared, no third party is involved. It is a question about work: "this will
-- process your whole library once." Wording it as a privacy agreement would be
-- both inaccurate and more alarming than the truth.
--
-- TURNING IT OFF KEEPS WHAT EXISTS
--
-- Disabling stops new work; it does not erase measurements. Deleting them would
-- mean re-fetching the whole library if the user changed their mind, which is
-- the one genuinely expensive thing in this feature. The cost is that "off"
-- does not mean "erased", and the UI should say so rather than imply otherwise.

ALTER TABLE families
    ADD COLUMN audio_analysis_enabled    BOOLEAN NOT NULL DEFAULT false,
    -- Who turned it on and when. Not an audit requirement — it is what lets a
    -- second admin see that a pass is already someone else's doing rather than
    -- starting another.
    ADD COLUMN audio_analysis_enabled_at TIMESTAMPTZ,
    ADD COLUMN audio_analysis_enabled_by UUID REFERENCES users (id) ON DELETE SET NULL;

-- The sweep asks "which families want this?" on every tick, and almost none
-- will once this is a real deployment.
CREATE INDEX families_audio_analysis_idx
    ON families (id)
    WHERE audio_analysis_enabled;
