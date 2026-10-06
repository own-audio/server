// SPDX-License-Identifier: AGPL-3.0-or-later
/// Synthetic ID3 ids for the OpenSubsonic browsing endpoints.
///
/// `music_tracks` stores `artist`/`album` as denormalized strings with no
/// backing tables, so there is no natural artist/album UUID to hand out.
/// Instead we derive a stable UUID v5 per (user, artist) and
/// (user, artist, album) tuple — deterministic across requests, unique per
/// user, and requires no schema change. Track ids are simply the real
/// `music_tracks.id`.
use uuid::Uuid;

pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub const UNKNOWN_ALBUM: &str = "Unknown Album";

// Fixed, arbitrary namespace — any stable constant works; it just needs to
// never change once clients have cached ids derived from it.
const NAMESPACE: Uuid = Uuid::from_bytes([
    0x6f, 0x1d, 0x1b, 0x6a, 0x4b, 0x1a, 0x4e, 0x9a, 0x8b, 0x0a, 0x2f, 0x7a, 0x9c, 0x8d, 0x0e, 0x11,
]);

pub fn artist_id(user_id: Uuid, artist: &str) -> Uuid {
    let key = format!("{user_id}|artist|{}", normalize(artist));
    Uuid::new_v5(&NAMESPACE, key.as_bytes())
}

pub fn album_id(user_id: Uuid, artist: &str, album: &str) -> Uuid {
    let key = format!("{user_id}|album|{}|{}", normalize(artist), normalize(album));
    Uuid::new_v5(&NAMESPACE, key.as_bytes())
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}
