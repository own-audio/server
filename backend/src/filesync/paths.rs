// SPDX-License-Identifier: AGPL-3.0-or-later
// Row tuples mirror the SELECT lists one-to-one; naming each a type would hide that.
#![allow(clippy::type_complexity)]
/// Where each item sits in the own.audio folder — docs/file-sync-plan.md §4.1
/// and §5.8, migration 0078.
///
/// The path belongs to the user. A path a client sends (a file put in the
/// Finder folder) is kept as given; an item created without one gets a default
/// from its metadata **once**, and nothing changes it afterwards — not a
/// metadata edit, not Identify. The only rename the system ever makes is the
/// ` (2)` that settles two items claiming the same path.
///
/// Paths are unique per owner across kinds, compared case-insensitively
/// (macOS and Windows both are), and only among live items: a file deleted in
/// Finder must not block a new file of the same name.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

pub const AUDIOBOOKS: &str = "Audiobooks";
pub const MUSIC: &str = "Music";
pub const PODCASTS: &str = "Podcasts";

const MAX_COMPONENT_BYTES: usize = 255;

/// Kinds that have a path, as `sync_paths.kind` spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Audiobook,
    MusicTrack,
    PodcastEpisode,
    /// An image, booklet, lyrics or cue sheet next to the audio (§2 item 16).
    CompanionFile,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Audiobook => "audiobook",
            Kind::MusicTrack => "music_track",
            Kind::PodcastEpisode => "podcast_episode",
            Kind::CompanionFile => "companion_file",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "audiobook" => Some(Kind::Audiobook),
            "music_track" => Some(Kind::MusicTrack),
            "podcast_episode" => Some(Kind::PodcastEpisode),
            "companion_file" => Some(Kind::CompanionFile),
            _ => None,
        }
    }

}

/// A track's or episode's path names a file; a book's names its folder —
/// except a book made of one loose file, whose path is that file and whose
/// only file has an empty relative path.
async fn names_file(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, kind: Kind, item: Uuid) -> anyhow::Result<bool> {
    if kind != Kind::Audiobook {
        return Ok(true);
    }
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM audiobook_files WHERE book_id = $1 AND relative_path = '')")
        .bind(item)
        .fetch_one(&mut **tx)
        .await
        .context("db: is the book one loose file")
}

// ── Names ─────────────────────────────────────────────────────────────────

/// One default path component, made safe for every platform the folder will
/// reach: none of `<>:"/\|?*` or control characters, no leading or trailing
/// space, no trailing dot, not a reserved Windows name, at most 255 bytes.
/// Only for names the server makes up — a user's own names are kept.
pub fn safe_component(raw: &str, fallback: &str) -> String {
    let nfc: String = raw.nfc().collect();
    let mut out = String::with_capacity(nfc.len());
    for c in nfc.replace(": ", " - ").chars() {
        match c {
            '/' | '\\' | ':' => out.push('-'),
            '"' => out.push('\''),
            '<' | '>' | '|' | '?' | '*' => out.push('_'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let mut name = out.trim().trim_end_matches('.').trim_end().to_string();
    if name.is_empty() {
        name = fallback.to_string();
    }
    if is_reserved(&name) {
        name.push('_');
    }
    truncate_bytes(&name, MAX_COMPONENT_BYTES).to_string()
}

fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end()
}

/// A file name from a stem and an extension, the stem shortened so the whole
/// stays within 255 bytes.
fn file_name(stem: &str, ext: &str) -> String {
    let room = MAX_COMPONENT_BYTES.saturating_sub(ext.len() + 1);
    format!("{}.{ext}", truncate_bytes(stem, room))
}

/// The file extension for a stored object: the key's own, else one for its
/// content type.
pub fn extension(object_key: &str, content_type: &str) -> String {
    let name = object_key.rsplit('/').next().unwrap_or(object_key);
    if let Some((_, ext)) = name.rsplit_once('.') {
        if (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return ext.to_ascii_lowercase();
        }
    }
    match content_type.split(';').next().unwrap_or("").trim() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" => "m4a",
        "audio/x-m4b" | "audio/m4b" => "m4b",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/ogg" | "application/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/webm" => "webm",
        _ => "bin",
    }
    .to_string()
}

fn padded(n: i64, count: i64) -> String {
    let width = count.max(1).to_string().len().max(2);
    format!("{n:0width$}")
}

// ── Default paths (§4.1) ──────────────────────────────────────────────────

/// `Audiobooks/<Author>/<Title>` — a book's folder.
pub fn default_book(author: Option<&str>, title: &str) -> String {
    format!(
        "{AUDIOBOOKS}/{}/{}",
        safe_component(author.unwrap_or(""), "Unknown Author"),
        safe_component(title, "Untitled")
    )
}

/// `<NN> - <File title>.<ext>` — a file inside a book's folder.
pub fn default_book_file(position: i64, count: i64, title: Option<&str>, ext: &str) -> String {
    let n = padded(position, count);
    let stem = match title.map(str::trim).map(|t| strip_own_number(t, position)).filter(|t| !t.is_empty()) {
        Some(t) => format!("{n} - {}", safe_component(t, "")),
        None => n,
    };
    file_name(&stem, ext)
}

/// A file title that already starts with its own position (`06 - Chapter`, `6. Chapter`)
/// loses it, so the default name does not number it twice. Any other leading number is
/// part of the title (`1984`, `2 Fast`).
fn strip_own_number(title: &str, position: i64) -> &str {
    let digits = title.len() - title.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || title[..digits].parse::<i64>().ok() != Some(position) {
        return title;
    }
    let rest = &title[digits..];
    if rest.trim().is_empty() {
        return "";
    }
    let stripped = rest.trim_start_matches([' ', '-', '.', '_', ')']);
    if stripped.len() == rest.len() { title } else { stripped }
}

/// The disc a folder named `CD 2`, `CD02`, `Disc 3` or `Disk 1` stands for.
pub fn disc_from_folder(dir: &str) -> Option<i32> {
    let name = dir.rsplit('/').next()?.trim().to_lowercase();
    let rest = ["cd", "disc", "disk"].iter().find_map(|p| name.strip_prefix(p))?;
    let digits = rest.trim_start_matches([' ', '_', '-', '.']);
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| digits.parse::<i32>().ok())
        .flatten()
        .filter(|n| *n > 0)
}

/// `Music/<Album artist>/<Album>/<NN> - <Title>.<ext>`, and `…/<Album>/CD <n>/…` for a track
/// of a multi-disc album — `disc` is set by the caller only then, so a one-disc album keeps
/// its tracks together.
pub fn default_track(
    album_artist: Option<&str>,
    album: Option<&str>,
    disc: Option<i32>,
    track_number: Option<i32>,
    title: &str,
    ext: &str,
) -> String {
    let title = safe_component(title, "Untitled");
    let stem = match track_number.filter(|n| *n > 0) {
        Some(n) => format!("{n:02} - {title}"),
        None => title,
    };
    let disc = disc.filter(|d| *d > 0).map(|d| format!("CD {d}/")).unwrap_or_default();
    format!(
        "{MUSIC}/{}/{}/{disc}{}",
        safe_component(album_artist.unwrap_or(""), "Unknown Artist"),
        safe_component(album.unwrap_or(""), "Unknown Album"),
        file_name(&stem, ext)
    )
}

/// `Podcasts/<Show>/<YYYY-MM-DD> - <Episode title>.<ext>`, dated by
/// publication, else by the day it was stored.
pub fn default_episode(show: &str, date: DateTime<Utc>, title: &str, ext: &str) -> String {
    let stem = format!("{} - {}", date.format("%Y-%m-%d"), safe_component(title, "Untitled"));
    format!("{PODCASTS}/{}/{}", safe_component(show, "Unknown Show"), file_name(&stem, ext))
}

// ── Paths a client sends ──────────────────────────────────────────────────

/// Check a path a client sent and bring it to the stored form (NFC). Names are
/// otherwise kept exactly as given.
pub fn normalize(raw: &str, kind: Kind) -> Result<String, String> {
    let path: String = raw.nfc().collect();
    let parts: Vec<&str> = path.split('/').collect();
    let tops: &[&str] = match kind {
        Kind::Audiobook => &[AUDIOBOOKS],
        Kind::MusicTrack => &[MUSIC],
        Kind::PodcastEpisode => &[PODCASTS],
        Kind::CompanionFile => &[MUSIC, AUDIOBOOKS],
    };
    if !parts.first().is_some_and(|p| tops.contains(p)) {
        return Err(format!("the path must start with '{}/'", tops.join("/' or '")));
    }
    if parts.len() < 2 {
        return Err("the path needs a name below the top-level folder".to_string());
    }
    for part in &parts {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err("the path has an empty, '.' or '..' component".to_string());
        }
        if part.len() > MAX_COMPONENT_BYTES {
            return Err("a path component is longer than 255 bytes".to_string());
        }
        if part.chars().any(char::is_control) {
            return Err("the path contains a control character".to_string());
        }
    }
    Ok(path)
}

/// Check a file's path inside a book folder.
pub fn normalize_relative(raw: &str) -> Result<String, String> {
    let path: String = raw.nfc().collect();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("'{raw}' has an empty, '.' or '..' component"));
        }
        if part.len() > MAX_COMPONENT_BYTES {
            return Err(format!("'{raw}' has a component longer than 255 bytes"));
        }
        if part.chars().any(char::is_control) {
            return Err(format!("'{raw}' contains a control character"));
        }
    }
    Ok(path)
}

/// Companion file types kept next to the audio (§2 item 16).
pub const COMPANION_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "pdf", "lrc", "cue", "txt", "nfo"];
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

pub fn file_extension(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_companion(name: &str) -> bool {
    COMPANION_EXTENSIONS.contains(&file_extension(name).as_str())
}

pub fn is_image(name: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&file_extension(name).as_str())
}

/// The folder part of a path (`Music/A/b.jpg` → `Music/A`).
pub fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// `name (n).ext` for a file, `name (n)` for a folder.
fn with_suffix(path: &str, n: u32, names_file: bool) -> String {
    let (dir, last) = match path.rsplit_once('/') {
        Some((d, l)) => (Some(d), l),
        None => (None, path),
    };
    let renamed = match (names_file, last.rsplit_once('.')) {
        (true, Some((stem, ext))) if !stem.is_empty() => file_name(&format!("{stem} ({n})"), ext),
        _ => format!("{} ({n})", truncate_bytes(last, MAX_COMPONENT_BYTES - 6)),
    };
    match dir {
        Some(d) => format!("{d}/{renamed}"),
        None => renamed,
    }
}

// ── Claiming a path ───────────────────────────────────────────────────────

/// Give an item its path, once. Returns the item's path: the existing one if
/// it already has one, else `wanted`, or `wanted (2)`, `(3)`… when another of
/// the owner's live items holds that path or one nested with it.
pub async fn claim(pool: &PgPool, owner: Uuid, kind: Kind, item: Uuid, wanted: &str) -> anyhow::Result<String> {
    let mut tx = pool.begin().await.context("db: begin path claim")?;
    // One owner's claims run one at a time, so two uploads of the same name
    // cannot both find it free.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('sync_paths:' || $1::text))")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .context("db: lock owner paths")?;

    let existing: Option<String> =
        sqlx::query_scalar("SELECT path FROM sync_paths WHERE kind = $1 AND item_id = $2")
            .bind(kind.as_str())
            .bind(item)
            .fetch_optional(&mut *tx)
            .await
            .context("db: find path")?;
    if let Some(path) = existing {
        tx.commit().await.context("db: commit path claim")?;
        return Ok(path);
    }

    let path = free_path(&mut tx, owner, kind, item, wanted).await?;
    sqlx::query("INSERT INTO sync_paths (kind, item_id, user_id, path) VALUES ($1, $2, $3, $4)")
        .bind(kind.as_str())
        .bind(item)
        .bind(owner)
        .bind(&path)
        .execute(&mut *tx)
        .await
        .context("db: insert path")?;
    tx.commit().await.context("db: commit path claim")?;
    Ok(path)
}

/// After a restore: if a live item took this item's path while it was in the
/// trash, the restored one moves to ` (2)` — the newer file keeps its name.
pub async fn settle_after_restore(pool: &PgPool, kind: Kind, item: Uuid) -> anyhow::Result<()> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT user_id, path FROM sync_paths WHERE kind = $1 AND item_id = $2")
            .bind(kind.as_str())
            .bind(item)
            .fetch_optional(pool)
            .await
            .context("db: find restored path")?;
    let Some((owner, path)) = row else { return Ok(()) };

    let mut tx = pool.begin().await.context("db: begin path settle")?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('sync_paths:' || $1::text))")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .context("db: lock owner paths")?;
    let free = free_path(&mut tx, owner, kind, item, &path).await?;
    if free != path {
        sqlx::query("UPDATE sync_paths SET path = $3 WHERE kind = $1 AND item_id = $2")
            .bind(kind.as_str())
            .bind(item)
            .bind(&free)
            .execute(&mut *tx)
            .await
            .context("db: move restored path")?;
    }
    tx.commit().await.context("db: commit path settle")?;
    Ok(())
}

pub(crate) async fn free_path(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: Uuid,
    kind: Kind,
    item: Uuid,
    wanted: &str,
) -> anyhow::Result<String> {
    let is_file = names_file(tx, kind, item).await?;
    let mut candidate = wanted.to_string();
    for n in 2..10_000 {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM sync_live_paths
                 WHERE user_id = $1 AND NOT (kind = $2 AND item_id = $3)
                   AND (lower(path) = lower($4)
                        -- Nesting is refused between items (no book inside a
                        -- book); a companion file lives inside a book folder
                        -- by design.
                        OR ($2 <> 'companion_file' AND kind <> 'companion_file'
                            AND (starts_with(lower(path), lower($4) || '/')
                                 OR starts_with(lower($4), lower(path) || '/')))))",
        )
        .bind(owner)
        .bind(kind.as_str())
        .bind(item)
        .bind(&candidate)
        .fetch_one(&mut **tx)
        .await
        .context("db: check path")?;
        if !taken {
            return Ok(candidate);
        }
        candidate = with_suffix(wanted, n, is_file);
    }
    anyhow::bail!("no free path near '{wanted}'")
}

// ── Defaults for items created without a path ────────────────────────────

/// Give a book its default folder if it has none, and every file in it a
/// name if it has none. Returns the folder, or `None` when the book is not
/// live.
pub async fn ensure_book(pool: &PgPool, book: Uuid) -> anyhow::Result<Option<String>> {
    let row: Option<(Uuid, String, Option<String>)> =
        sqlx::query_as("SELECT user_id, title, author FROM audiobook_books WHERE id = $1")
            .bind(book)
            .fetch_optional(pool)
            .await
            .context("db: load book for path")?;
    let Some((owner, title, author)) = row else { return Ok(None) };
    let folder = claim(pool, owner, Kind::Audiobook, book, &default_book(author.as_deref(), &title)).await?;
    ensure_book_files(pool, book).await?;
    Ok(Some(folder))
}

/// Name the files of a book that have no name yet, `<NN> - <title>.<ext>`,
/// never repeating a name already in the folder.
pub async fn ensure_book_files(pool: &PgPool, book: Uuid) -> anyhow::Result<()> {
    let files: Vec<(Uuid, i32, Option<String>, Option<String>, String, String)> = sqlx::query_as(
        "SELECT f.id, f.position, f.title, f.relative_path, m.object_key, m.content_type
           FROM audiobook_files f JOIN media_objects m ON m.id = f.audio_object_id
          WHERE f.book_id = $1 ORDER BY f.position",
    )
    .bind(book)
    .fetch_all(pool)
    .await
    .context("db: load book files for paths")?;
    if files.iter().all(|f| f.3.is_some()) {
        return Ok(());
    }

    let count = files.len() as i64;
    let mut taken: std::collections::HashSet<String> =
        files.iter().filter_map(|f| f.3.as_deref().map(str::to_lowercase)).collect();
    for (id, position, title, relative, key, content_type) in &files {
        if relative.is_some() {
            continue;
        }
        let wanted = default_book_file(*position as i64, count, title.as_deref(), &extension(key, content_type));
        let mut name = wanted.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            name = with_suffix(&wanted, n, true);
            n += 1;
        }
        taken.insert(name.to_lowercase());
        sqlx::query("UPDATE audiobook_files SET relative_path = $2 WHERE id = $1 AND relative_path IS NULL")
            .bind(id)
            .bind(&name)
            .execute(pool)
            .await
            .context("db: name book file")?;
    }
    Ok(())
}

/// Give a track its default path if it has none. `None` when not live.
pub async fn ensure_track(pool: &PgPool, track: Uuid) -> anyhow::Result<Option<String>> {
    let sql = format!(
        "SELECT {}, mo.object_key, mo.content_type
           FROM music_tracks t JOIN media_objects mo ON mo.id = t.audio_object_id
          WHERE t.id = $1",
        crate::db::music::TRACK_COLS_WITH_CHECKSUM
    );
    let row = sqlx::query(&sql).bind(track).fetch_optional(pool).await.context("db: load track for path")?;
    let Some(row) = row else { return Ok(None) };
    use sqlx::{FromRow, Row};
    let t = crate::music::models::MusicTrack::from_row(&row).context("db: decode track")?;
    let key: String = row.try_get("object_key")?;
    let content_type: String = row.try_get("content_type")?;
    let album_artist = t.effective_album_artist();
    let multi_disc = t.disc_total.is_some_and(|n| n > 1) || t.disc_number.is_some_and(|n| n > 1);
    let wanted = default_track(
        album_artist.as_deref(),
        t.album.as_deref(),
        t.disc_number.filter(|_| multi_disc),
        t.track_number,
        &t.title,
        &extension(&key, &content_type),
    );
    claim(pool, t.user_id, Kind::MusicTrack, track, &wanted).await.map(Some)
}

/// Give a stored episode its default path if it has none. `None` when the
/// episode is not stored.
pub async fn ensure_episode(pool: &PgPool, episode: Uuid) -> anyhow::Result<Option<String>> {
    let row: Option<(Uuid, String, String, Option<DateTime<Utc>>, DateTime<Utc>, String, String)> = sqlx::query_as(
        "SELECT f.user_id, f.title, e.title, e.published_at, m.created_at, m.object_key, m.content_type
           FROM podcast_episodes e
           JOIN podcast_feeds f ON f.id = e.feed_id
           JOIN media_objects m ON m.id = e.audio_object_id
          WHERE e.id = $1",
    )
    .bind(episode)
    .fetch_optional(pool)
    .await
    .context("db: load episode for path")?;
    let Some((owner, show, title, published, stored, key, content_type)) = row else { return Ok(None) };
    let wanted = default_episode(&show, published.unwrap_or(stored), &title, &extension(&key, &content_type));
    claim(pool, owner, Kind::PodcastEpisode, episode, &wanted).await.map(Some)
}

/// Default paths for whichever of these live items have none yet — the
/// backfill of items that predate paths, and the safety net for any creating
/// route that did not set one.
pub async fn ensure_all(pool: &PgPool, items: &[(Kind, Uuid)]) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let kinds: Vec<&str> = items.iter().map(|(k, _)| k.as_str()).collect();
    let ids: Vec<Uuid> = items.iter().map(|(_, id)| *id).collect();
    let missing: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT w.kind, w.id FROM unnest($1::text[], $2::uuid[]) AS w(kind, id)
          WHERE NOT EXISTS (SELECT 1 FROM sync_paths p WHERE p.kind = w.kind AND p.item_id = w.id)
             OR (w.kind = 'audiobook' AND EXISTS (
                    SELECT 1 FROM audiobook_files f WHERE f.book_id = w.id AND f.relative_path IS NULL))",
    )
    .bind(&kinds)
    .bind(&ids)
    .fetch_all(pool)
    .await
    .context("db: find items without a path")?;
    for (kind, id) in missing {
        let result = match Kind::parse(&kind) {
            Some(Kind::Audiobook) => ensure_book(pool, id).await,
            Some(Kind::MusicTrack) => ensure_track(pool, id).await,
            Some(Kind::PodcastEpisode) => ensure_episode(pool, id).await,
            // Companion files always come with their path.
            Some(Kind::CompanionFile) | None => Ok(None),
        };
        if let Err(e) = result {
            tracing::warn!(%kind, %id, "could not give the item a path: {e:#}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_work_everywhere() {
        assert_eq!(safe_component("AC/DC", "x"), "AC-DC");
        assert_eq!(safe_component("Book: A Subtitle", "x"), "Book - A Subtitle");
        assert_eq!(safe_component("What?*", "x"), "What__");
        assert_eq!(safe_component("  trailing dots... ", "x"), "trailing dots");
        assert_eq!(safe_component("", "Unknown Album"), "Unknown Album");
        assert_eq!(safe_component("con", "x"), "con_");
        assert_eq!(safe_component("LPT1.txt", "x"), "LPT1.txt_");
        assert_eq!(safe_component("COM0", "x"), "COM0");
        assert_eq!(safe_component("say \"hi\"", "x"), "say 'hi'");
        let long = "é".repeat(200);
        assert!(safe_component(&long, "x").len() <= 255);
    }

    #[test]
    fn names_are_stored_composed() {
        // macOS reports "é" as e + combining acute.
        let decomposed = "Cafe\u{301}";
        assert_eq!(safe_component(decomposed, "x"), "Café");
        assert_eq!(normalize(&format!("Music/{decomposed}.mp3"), Kind::MusicTrack).unwrap(), "Music/Café.mp3");
    }

    #[test]
    fn default_layout() {
        assert_eq!(default_book(Some("Terry Pratchett"), "Mort"), "Audiobooks/Terry Pratchett/Mort");
        assert_eq!(default_book(None, "Mort"), "Audiobooks/Unknown Author/Mort");
        assert_eq!(default_book_file(3, 12, Some("Chapter 3"), "mp3"), "03 - Chapter 3.mp3");
        assert_eq!(default_book_file(7, 140, None, "m4b"), "007.m4b");
        assert_eq!(default_book_file(6, 60, Some("06 - I Jak spadla lavina"), "mp3"), "06 - I Jak spadla lavina.mp3");
        assert_eq!(default_book_file(6, 60, Some("6. Kapitola"), "mp3"), "06 - Kapitola.mp3");
        assert_eq!(default_book_file(1, 9, Some("1984"), "mp3"), "01 - 1984.mp3", "a number that is not its position stays");
        assert_eq!(default_book_file(2, 9, Some("2 Fast"), "mp3"), "02 - Fast.mp3");
        assert_eq!(default_book_file(3, 9, Some("03"), "mp3"), "03.mp3");
        assert_eq!(
            default_track(Some("Queen"), Some("A Night at the Opera"), None, Some(11), "Bohemian Rhapsody", "flac"),
            "Music/Queen/A Night at the Opera/11 - Bohemian Rhapsody.flac"
        );
        assert_eq!(default_track(None, None, None, None, "x", "mp3"), "Music/Unknown Artist/Unknown Album/x.mp3");
        assert_eq!(
            default_track(Some("Amy Winehouse"), Some("At The BBC"), Some(2), Some(1), "Know You Now", "flac"),
            "Music/Amy Winehouse/At The BBC/CD 2/01 - Know You Now.flac"
        );
        assert_eq!(disc_from_folder("Music/Box/CD 01"), Some(1));
        assert_eq!(disc_from_folder("Music/Box/Disc 3"), Some(3));
        assert_eq!(disc_from_folder("Music/Box/CDs"), None);
        let day = DateTime::parse_from_rfc3339("2026-09-25T10:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(
            default_episode("Radio: Wave", day, "Ep 1", "mp3"),
            "Podcasts/Radio - Wave/2026-09-25 - Ep 1.mp3"
        );
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("f/x/uploads/music/abc/01-song.FLAC", "audio/flac"), "flac");
        assert_eq!(extension("f/x/episodes/123", "audio/mpeg"), "mp3");
        assert_eq!(extension("f/x/episodes/123", "audio/mp4; codecs=mp4a"), "m4a");
        assert_eq!(extension("f/x/y", "application/octet-stream"), "bin");
    }

    #[test]
    fn client_paths_are_checked_not_rewritten() {
        assert_eq!(normalize("Music/Moje oblíbené/track01.mp3", Kind::MusicTrack).unwrap(), "Music/Moje oblíbené/track01.mp3");
        assert_eq!(normalize("Music/a:b?.mp3", Kind::MusicTrack).unwrap(), "Music/a:b?.mp3");
        assert!(normalize("Audiobooks/x.mp3", Kind::MusicTrack).is_err());
        assert!(normalize("Music", Kind::MusicTrack).is_err());
        assert!(normalize("Music//x.mp3", Kind::MusicTrack).is_err());
        assert!(normalize("Music/../x.mp3", Kind::MusicTrack).is_err());
        assert!(normalize_relative("CD1/01.mp3").is_ok());
        assert!(normalize("Audiobooks/A/cover.jpg", Kind::CompanionFile).is_ok());
        assert!(normalize("Music/A/cover.jpg", Kind::CompanionFile).is_ok());
        assert!(normalize("Podcasts/A/cover.jpg", Kind::CompanionFile).is_err());
        assert!(is_companion("Music/A/Booklet.PDF") && is_image("x/Front.JPG") && !is_companion("a.zip"));
        assert!(normalize_relative("/01.mp3").is_err());
    }

    #[test]
    fn suffixes() {
        assert_eq!(with_suffix("Music/A/song.mp3", 2, true), "Music/A/song (2).mp3");
        assert_eq!(with_suffix("Audiobooks/A/Mort", 2, false), "Audiobooks/A/Mort (2)");
        assert_eq!(with_suffix("Audiobooks/A/Vol. 2", 3, false), "Audiobooks/A/Vol. 2 (3)");
        assert_eq!(with_suffix("Music/A/.hidden", 2, true), "Music/A/.hidden (2)");
    }
}
