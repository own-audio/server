-- Allow 'tvos' as a device_kind, for the Apple TV client (audio2-tvos).
--
-- Same shape as 0064 (windows) and 0027 (macos). The value is gated in five
-- places that must agree: three Rust allowlists (auth::issue_tokens,
-- playback::normalize_device_kind, playback::normalize_eq_device_kind) and the
-- three CHECK constraints below. Relaxing only the Rust side makes login 500 on
-- a constraint violation rather than falling back to 'other'.

ALTER TABLE refresh_tokens
    DROP CONSTRAINT refresh_tokens_device_kind_check;

ALTER TABLE refresh_tokens
    ADD CONSTRAINT refresh_tokens_device_kind_check
    CHECK (device_kind IN ('web', 'ios', 'android', 'macos', 'windows', 'tvos', 'other'));

ALTER TABLE listening_sessions
    DROP CONSTRAINT listening_sessions_device_kind_check;

ALTER TABLE listening_sessions
    ADD CONSTRAINT listening_sessions_device_kind_check
    CHECK (device_kind IN ('web', 'ios', 'android', 'macos', 'windows', 'tvos', 'subsonic', 'other'));

ALTER TABLE eq_settings
    DROP CONSTRAINT eq_settings_device_kind_check;

ALTER TABLE eq_settings
    ADD CONSTRAINT eq_settings_device_kind_check
    CHECK (device_kind IN ('web', 'ios', 'android', 'macos', 'windows', 'tvos', 'other'));
