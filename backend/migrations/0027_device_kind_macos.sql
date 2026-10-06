-- Allow 'macos' as a device_kind.
--
-- The value is gated in four places that must agree: two Rust allowlists
-- (auth::issue_tokens, playback::normalize_device_kind) and the two CHECK
-- constraints below. Relaxing only the Rust side makes login 500 on a
-- constraint violation rather than falling back to 'other', because the
-- allowlist is what *stops* an unknown value from ever reaching the insert.
--
-- The two lists stay deliberately different: 'subsonic' is a playback source
-- and never a login, so it belongs on listening_sessions only.

ALTER TABLE refresh_tokens
    DROP CONSTRAINT refresh_tokens_device_kind_check;

ALTER TABLE refresh_tokens
    ADD CONSTRAINT refresh_tokens_device_kind_check
    CHECK (device_kind IN ('web', 'ios', 'android', 'macos', 'other'));

ALTER TABLE listening_sessions
    DROP CONSTRAINT listening_sessions_device_kind_check;

ALTER TABLE listening_sessions
    ADD CONSTRAINT listening_sessions_device_kind_check
    CHECK (device_kind IN ('web', 'ios', 'android', 'macos', 'subsonic', 'other'));
