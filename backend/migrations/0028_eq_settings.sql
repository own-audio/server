-- Per-device equalizer settings.
--
-- One row per (user, device_kind): a phone's speakers and a Mac's speakers
-- want different curves, so this is not folded into the single-row
-- user_settings table. Bands are positional (band1..band6), not named after
-- their frequency in the column itself — `band_2_4k_db` round-trips through
-- Swift's `.convertFromSnakeCase` as the near-unreadable `band24KDb`, so the
-- fixed 60/150/400/1k/2.4k/15k Hz centers this graphic EQ actually uses live
-- as a client-side constant instead (see `EqBand.standardFrequencies` in the
-- shared Swift package). Gains in dB, clamped server-side to [-12, 12].
-- `subsonic` is excluded from the device_kind check — EQ is a first-party
-- client feature, not something a generic Subsonic API client would set
-- (mirrors refresh_tokens' allowlist, not listening_sessions').
CREATE TABLE eq_settings (
    user_id      UUID             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_kind  TEXT             NOT NULL CHECK (device_kind IN ('web', 'ios', 'android', 'macos', 'other')),
    enabled      BOOLEAN          NOT NULL DEFAULT false,
    preamp_db    DOUBLE PRECISION NOT NULL DEFAULT 0,
    band1_db     DOUBLE PRECISION NOT NULL DEFAULT 0,
    band2_db     DOUBLE PRECISION NOT NULL DEFAULT 0,
    band3_db     DOUBLE PRECISION NOT NULL DEFAULT 0,
    band4_db     DOUBLE PRECISION NOT NULL DEFAULT 0,
    band5_db     DOUBLE PRECISION NOT NULL DEFAULT 0,
    band6_db     DOUBLE PRECISION NOT NULL DEFAULT 0,
    preset_name  TEXT,
    updated_at   TIMESTAMPTZ      NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, device_kind)
);
