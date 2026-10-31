-- What a stored file's audio is, read from its headers when it arrives
-- (src/audio_props.rs). NULL everywhere for files stored before this, until a
-- folder scan or a new upload reads them; `audio_probed_at` marks a file
-- already read, so one nothing can read is not read again on every scan.
ALTER TABLE media_objects
    ADD COLUMN audio_codec text,
    ADD COLUMN audio_sample_rate_hz integer,
    ADD COLUMN audio_bit_depth smallint,
    ADD COLUMN audio_channels smallint,
    ADD COLUMN audio_bitrate_kbps integer,
    ADD COLUMN audio_probed_at timestamptz;
