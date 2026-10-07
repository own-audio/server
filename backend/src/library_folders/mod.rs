// SPDX-License-Identifier: AGPL-3.0-or-later
//! Read-only library folders: music and audiobook collections already on a
//! disk or NAS share, indexed where they are (Phase 4 A, issue #1).
//!
//! Folders come from configuration (`LIBRARY__*`) and are remembered in
//! `library_folders`, so their ids — and with them every file's key
//! `folder/<id>/<relative path>` — stay stable. The object store reads those
//! keys straight from the folder and serves them through the media route
//! (`storage::FOLDER_PREFIX`); nothing is ever copied or written there.
//!
//! The scanner walks one directory at a time and remembers every file's size
//! and modification time in `library_files`, so memory does not grow with the
//! catalog and a rescan only reads what changed. It runs at start, every
//! `LIBRARY__SCAN_INTERVAL_SECS` (hourly), and on request.

use crate::app::AppState;
use crate::families::FamilyContext;
use anyhow::Context;
use serde::Deserialize;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use uuid::Uuid;

pub mod routes;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "m4b", "aac", "flac", "ogg", "oga", "opus", "wav", "aif", "aiff", "wma"];
const COVER_NAMES: &[&str] = &["cover.jpg", "cover.jpeg", "cover.png", "folder.jpg", "folder.jpeg", "folder.png", "front.jpg", "front.png"];
const DEFAULT_INTERVAL_SECS: u64 = 3600;

/// Wakes the scanner early (admin "scan now", Subsonic `startScan`).
static SCAN_NOW: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);
static SCANNING: AtomicBool = AtomicBool::new(false);
/// Files looked at in the scan that is running, or the last one.
static SCANNED: AtomicU64 = AtomicU64::new(0);

/// Ask for a scan now; it starts as soon as any running scan has finished.
pub fn request_scan() {
    SCAN_NOW.notify_one();
}

/// `(scanning, files looked at)` — what Subsonic's `getScanStatus` reports.
pub fn scan_status() -> (bool, u64) {
    (SCANNING.load(Ordering::Relaxed), SCANNED.load(Ordering::Relaxed))
}

#[derive(Debug, Clone, Deserialize)]
struct FolderSpec {
    path: String,
    kind: String,
    #[serde(default)]
    visibility: Option<String>,
}

/// The folders configuration asks for, in order; empty when none.
pub fn configured(cfg: &crate::app::AppConfig) -> anyhow::Result<Vec<(String, String, String)>> {
    let Some(lib) = &cfg.library else { return Ok(Vec::new()) };
    let default_vis = lib.visibility.clone().unwrap_or_else(|| "family".to_string());
    let mut out = Vec::new();
    if let Some(p) = lib.music.as_deref().filter(|p| !p.trim().is_empty()) {
        out.push((p.trim().to_string(), "music".to_string(), default_vis.clone()));
    }
    if let Some(p) = lib.audiobooks.as_deref().filter(|p| !p.trim().is_empty()) {
        out.push((p.trim().to_string(), "audiobooks".to_string(), default_vis.clone()));
    }
    if let Some(json) = lib.folders.as_deref().filter(|j| !j.trim().is_empty()) {
        let specs: Vec<FolderSpec> = serde_json::from_str(json).context("LIBRARY__FOLDERS is not a JSON list of {path, kind}")?;
        for s in specs {
            anyhow::ensure!(s.kind == "music" || s.kind == "audiobooks", "library folder kind must be `music` or `audiobooks`, not `{}`", s.kind);
            out.push((s.path, s.kind, s.visibility.unwrap_or_else(|| default_vis.clone())));
        }
    }
    for (_, _, vis) in &out {
        anyhow::ensure!(vis == "family" || vis == "private", "library visibility must be `family` or `private`");
    }
    Ok(out)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FolderRow {
    pub id: Uuid,
    pub path: String,
    pub kind: String,
    pub family_id: Uuid,
    pub owner_id: Uuid,
    pub visibility: String,
}

/// Bring `library_folders` in line with configuration and register every
/// folder with the object store. Needs a first admin (it owns what the
/// folders hold); before setup it returns nothing and is called again later.
pub async fn sync_folders(state: &AppState) -> anyhow::Result<Vec<FolderRow>> {
    let wanted = configured(state.config())?;
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let owner: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT user_id, family_id FROM family_members
         WHERE role = 'family_admin' ORDER BY joined_at LIMIT 1",
    )
    .fetch_optional(state.db())
    .await
    .context("find the family's admin")?;
    let Some((owner_id, family_id)) = owner else {
        return Ok(Vec::new());
    };

    let mut rows = Vec::new();
    for (path, kind, visibility) in wanted {
        let row: FolderRow = sqlx::query_as(
            "INSERT INTO library_folders (path, kind, family_id, owner_id, visibility)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (path) DO UPDATE SET kind = EXCLUDED.kind, visibility = EXCLUDED.visibility
             RETURNING id, path, kind, family_id, owner_id, visibility",
        )
        .bind(&path)
        .bind(&kind)
        .bind(family_id)
        .bind(owner_id)
        .bind(&visibility)
        .fetch_one(state.db())
        .await
        .context("remember library folder")?;
        state.storage().register_folder(row.id, PathBuf::from(&row.path));
        rows.push(row);
    }
    Ok(rows)
}

/// The scanner loop. Started at boot when folders are configured.
pub fn spawn(state: AppState) {
    let interval = state
        .config()
        .library
        .as_ref()
        .and_then(|l| l.scan_interval_secs)
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(60);
    tokio::spawn(async move {
        loop {
            match sync_folders(&state).await {
                Ok(folders) if folders.is_empty() => {
                    // No admin yet (setup not done): look again soon.
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                        _ = SCAN_NOW.notified() => {}
                    }
                    continue;
                }
                Ok(folders) => {
                    SCANNING.store(true, Ordering::Relaxed);
                    SCANNED.store(0, Ordering::Relaxed);
                    for folder in &folders {
                        if let Err(e) = scan_folder(&state, folder).await {
                            tracing::warn!(folder = %folder.path, error = %format!("{e:#}"), "library scan failed");
                            let _ = sqlx::query("UPDATE library_folders SET scan_error = $2, scan_finished_at = now() WHERE id = $1")
                                .bind(folder.id)
                                .bind(format!("{e:#}"))
                                .execute(state.db())
                                .await;
                        }
                    }
                    SCANNING.store(false, Ordering::Relaxed);
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "library folders: configuration"),
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                _ = SCAN_NOW.notified() => {}
            }
        }
    });
}

/// One directory's contents: subdirectories, and files as (name, size, mtime).
type Listing = (Vec<String>, Vec<(String, u64, i64)>);

fn list_dir(dir: &Path) -> std::io::Result<Listing> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            dirs.push(name);
        } else if meta.is_file() {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            files.push((name, meta.len(), mtime));
        }
    }
    dirs.sort_by(|a, b| natural_cmp(b, a)); // popped from the end: ascending
    files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    Ok((dirs, files))
}

/// Order names the way people number files: `2` before `10`.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.to_lowercase().chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, buf)) if *d == digit => buf.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    for (x, y) in chunks(a).iter().zip(chunks(b).iter()) {
        let ord = match (x.0, y.0) {
            (true, true) => {
                let (xa, ya) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
                xa.len().cmp(&ya.len()).then_with(|| xa.cmp(ya))
            }
            _ => x.1.cmp(&y.1),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn is_audio(name: &str) -> bool {
    name.contains('.') && AUDIO_EXTENSIONS.contains(&extension(name).as_str())
}

fn audio_content_type(name: &str) -> &'static str {
    match extension(name).as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "m4b" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        "wma" => "audio/x-ms-wma",
        _ => "application/octet-stream",
    }
}

fn image_content_type(name: &str) -> &'static str {
    if extension(name) == "png" { "image/png" } else { "image/jpeg" }
}

/// Length in whole seconds and the title tag, from one read of the headers.
fn probe(path: &Path) -> (Option<i32>, Option<String>) {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;
    let Ok(file) = lofty::read_from_path(path) else { return (None, None) };
    let secs = file.properties().duration().as_secs();
    let title = file
        .primary_tag()
        .or_else(|| file.first_tag())
        .and_then(|t| t.title().map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty());
    ((secs > 0).then_some(secs.min(i32::MAX as u64) as i32), title)
}

fn duration_secs(path: &Path) -> Option<i32> {
    probe(path).0
}

/// Title for an audiobook file from its name: `03_The_Ride.mp3` → `03 The Ride`.
fn file_title(name: &str) -> String {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    stem.replace(['_', '.'], " ").trim().to_string()
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

/// The context the scanner acts in: the family admin who owns the folder.
fn owner_context(folder: &FolderRow) -> FamilyContext {
    FamilyContext {
        user_id: folder.owner_id,
        family_id: folder.family_id,
        family_role: "family_admin".to_string(),
        is_global_admin: false,
        can_upload: true,
        can_generate: false,
    }
}

struct Known {
    size: i64,
    mtime: i64,
    item_id: Option<Uuid>,
}

async fn known_files(pool: &PgPool, folder: Uuid, rels: &[String]) -> anyhow::Result<std::collections::HashMap<String, Known>> {
    let rows: Vec<(String, i64, i64, Option<Uuid>)> = sqlx::query_as(
        "SELECT rel_path, size_bytes, mtime_secs, item_id FROM library_files
         WHERE folder_id = $1 AND rel_path = ANY($2)",
    )
    .bind(folder)
    .bind(rels)
    .fetch_all(pool)
    .await
    .context("read known library files")?;
    Ok(rows
        .into_iter()
        .map(|(rel, size, mtime, item_id)| (rel, Known { size, mtime, item_id }))
        .collect())
}

async fn remember(pool: &PgPool, folder: Uuid, rel: &str, size: u64, mtime: i64, kind: &str, item: Option<Uuid>) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO library_files (folder_id, rel_path, size_bytes, mtime_secs, item_kind, item_id, missing, last_seen_at)
         VALUES ($1, $2, $3, $4, $5, $6, false, now())
         ON CONFLICT (folder_id, rel_path) DO UPDATE
           SET size_bytes = EXCLUDED.size_bytes, mtime_secs = EXCLUDED.mtime_secs,
               item_kind = COALESCE(EXCLUDED.item_kind, library_files.item_kind),
               item_id = COALESCE(EXCLUDED.item_id, library_files.item_id),
               missing = false, last_seen_at = now()",
    )
    .bind(folder)
    .bind(rel)
    .bind(size as i64)
    .bind(mtime)
    .bind(kind)
    .bind(item)
    .execute(pool)
    .await
    .context("remember library file")?;
    Ok(())
}

/// Walk one folder. Returns `(files looked at, items added)`.
pub async fn scan_folder(state: &AppState, folder: &FolderRow) -> anyhow::Result<(u64, u64)> {
    let pool = state.db();
    let root = PathBuf::from(&folder.path);
    anyhow::ensure!(root.is_dir(), "{} is not a folder the server can read", folder.path);
    let started: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE library_folders SET scan_started_at = now(), scan_error = NULL WHERE id = $1 RETURNING scan_started_at",
    )
    .bind(folder.id)
    .fetch_one(pool)
    .await?;

    let ctx = owner_context(folder);
    let family_id = (folder.visibility == "family").then_some(folder.family_id);
    let (mut seen, mut added) = (0u64, 0u64);
    let mut stack = vec![String::new()];

    while let Some(rel_dir) = stack.pop() {
        let dir = root.join(&rel_dir);
        let (subdirs, files) = match tokio::task::spawn_blocking(move || list_dir(&dir)).await? {
            Ok(listing) => listing,
            Err(e) => {
                tracing::warn!(dir = %rel_dir, error = %e, "library scan: cannot read folder");
                continue;
            }
        };
        for sub in subdirs {
            stack.push(join_rel(&rel_dir, &sub));
        }
        let cover = files
            .iter()
            .find(|(n, _, _)| COVER_NAMES.contains(&n.to_lowercase().as_str()))
            .map(|(n, size, _)| (join_rel(&rel_dir, n), image_content_type(n), *size));
        let audio: Vec<(String, String, u64, i64)> = files
            .into_iter()
            .filter(|(n, _, _)| is_audio(n))
            .map(|(n, size, mtime)| (join_rel(&rel_dir, &n), n, size, mtime))
            .collect();
        if audio.is_empty() {
            continue;
        }
        seen += audio.len() as u64;
        SCANNED.fetch_add(audio.len() as u64, Ordering::Relaxed);

        let rels: Vec<String> = audio.iter().map(|a| a.0.clone()).collect();
        let known = known_files(pool, folder.id, &rels).await?;
        let (unchanged, fresh): (Vec<_>, Vec<_>) = audio.into_iter().partition(|(rel, _, size, mtime)| {
            known.get(rel).is_some_and(|k| k.size == *size as i64 && k.mtime == *mtime)
        });

        if !unchanged.is_empty() {
            let rels: Vec<String> = unchanged.iter().map(|a| a.0.clone()).collect();
            sqlx::query(
                "UPDATE library_files SET last_seen_at = now(), missing = false
                 WHERE folder_id = $1 AND rel_path = ANY($2)",
            )
            .bind(folder.id)
            .bind(&rels)
            .execute(pool)
            .await?;
        }
        if fresh.is_empty() {
            continue;
        }

        match folder.kind.as_str() {
            "music" => {
                let cover_key = cover.as_ref().map(|(rel, ct, size)| (crate::storage::folder_key(folder.id, rel), *ct, *size as i64));
                for (rel, name, size, mtime) in fresh {
                    // A changed file keeps its track; only the bookkeeping moves.
                    if let Some(item) = known.get(&rel).and_then(|k| k.item_id) {
                        remember(pool, folder.id, &rel, size, mtime, "track", Some(item)).await?;
                        continue;
                    }
                    let path = root.join(&rel);
                    let key = crate::storage::folder_key(folder.id, &rel);
                    let p2 = path.clone();
                    let duration = tokio::task::spawn_blocking(move || duration_secs(&p2)).await.ok().flatten();
                    let created = crate::music::create_folder_track(
                        state,
                        &ctx,
                        family_id,
                        &path,
                        &key,
                        audio_content_type(&name),
                        size as i64,
                        duration,
                        cover_key.as_ref().map(|(k, ct, s)| (k.as_str(), *ct, *s)),
                    )
                    .await;
                    match created {
                        Ok(id) => {
                            remember(pool, folder.id, &rel, size, mtime, "track", Some(id)).await?;
                            added += 1;
                        }
                        Err(e) => tracing::warn!(file = %rel, error = %format!("{e:#}"), "library scan: track not added"),
                    }
                }
            }
            _ => {
                let book_added = scan_book_dir(state, folder, &ctx, family_id, &rel_dir, &known, fresh, cover).await?;
                added += book_added;
            }
        }
    }

    let missing: i64 = sqlx::query_scalar(
        "WITH m AS (
             UPDATE library_files SET missing = true
             WHERE folder_id = $1 AND last_seen_at < $2 AND NOT missing
             RETURNING 1)
         SELECT count(*) FROM m",
    )
    .bind(folder.id)
    .bind(started)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "UPDATE library_folders
         SET scan_finished_at = now(), files_seen = $2, files_added = $3, scan_error = NULL
         WHERE id = $1",
    )
    .bind(folder.id)
    .bind(seen as i32)
    .bind(added as i32)
    .execute(pool)
    .await?;
    tracing::info!(folder = %folder.path, seen, added, missing, "library scan finished");
    Ok((seen, added))
}

/// A folder of audio files is one book: its name the title, its parent's
/// name the author (`Author/Title/*.mp3`). New files join the book that the
/// folder's other files already belong to.
#[allow(clippy::too_many_arguments)]
async fn scan_book_dir(
    state: &AppState,
    folder: &FolderRow,
    ctx: &FamilyContext,
    family_id: Option<Uuid>,
    rel_dir: &str,
    known: &std::collections::HashMap<String, Known>,
    fresh: Vec<(String, String, u64, i64)>,
    cover: Option<(String, &'static str, u64)>,
) -> anyhow::Result<u64> {
    let pool = state.db();
    let root = PathBuf::from(&folder.path);

    // Files already known keep their place; only bookkeeping moves.
    let mut new_files = Vec::new();
    for f in fresh {
        match known.get(&f.0).and_then(|k| k.item_id) {
            Some(item) => remember(pool, folder.id, &f.0, f.2, f.3, "book_file", Some(item)).await?,
            None if known.contains_key(&f.0) => {
                // Known, its item removed by someone: stays hidden.
                remember(pool, folder.id, &f.0, f.2, f.3, "book_file", None).await?
            }
            None => new_files.push(f),
        }
    }
    if new_files.is_empty() {
        return Ok(0);
    }

    let existing_ids: Vec<Uuid> = known.values().filter_map(|k| k.item_id).collect();
    let existing_book: Option<Uuid> = if existing_ids.is_empty() {
        None
    } else {
        sqlx::query_scalar("SELECT book_id FROM audiobook_files WHERE id = ANY($1) LIMIT 1")
            .bind(&existing_ids)
            .fetch_optional(pool)
            .await?
    };

    let mut durations = Vec::with_capacity(new_files.len());
    let mut titles = Vec::with_capacity(new_files.len());
    for (rel, _, _, _) in &new_files {
        let path = root.join(rel);
        let (duration, title) = tokio::task::spawn_blocking(move || probe(&path)).await.unwrap_or((None, None));
        durations.push(duration);
        titles.push(title);
    }

    let mut tx = pool.begin().await?;
    let (book_id, mut position) = match existing_book {
        Some(id) => {
            let last: Option<i32> = sqlx::query_scalar("SELECT max(position) FROM audiobook_files WHERE book_id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
            (id, last.unwrap_or(0))
        }
        None => {
            let dir = Path::new(rel_dir);
            let title = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| file_title(&new_files[0].1));
            let author = dir
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|a| !a.is_empty());
            let total: Option<i32> = durations.iter().copied().flatten().reduce(|a, b| a + b);
            let book = crate::db::audiobooks::insert_book_full(
                &mut *tx,
                ctx.user_id,
                family_id,
                &title,
                author.as_deref(),
                None,
                None,
                total,
            )
            .await?;
            if let Some((cover_rel, ct, size)) = &cover {
                let key = crate::storage::folder_key(folder.id, cover_rel);
                let media = crate::db::media::upsert_object(&mut *tx, state.storage().bucket(), &key, ct, Some(*size as i64)).await?;
                sqlx::query("UPDATE audiobook_books SET cover_object_id = $2 WHERE id = $1")
                    .bind(book.id)
                    .bind(media)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("UPDATE audiobook_books_all SET source = 'folder' WHERE id = $1")
                .bind(book.id)
                .execute(&mut *tx)
                .await?;
            (book.id, 0)
        }
    };

    let mut file_ids = Vec::with_capacity(new_files.len());
    for (((rel, name, size, _), duration), title) in new_files.iter().zip(&durations).zip(&titles) {
        position += 1;
        let key = crate::storage::folder_key(folder.id, rel);
        let media = crate::db::media::upsert_object(&mut *tx, state.storage().bucket(), &key, audio_content_type(name), Some(*size as i64)).await?;
        let file_id: Uuid = sqlx::query_scalar(
            "INSERT INTO audiobook_files (book_id, position, title, duration_secs, audio_object_id, relative_path)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(book_id)
        .bind(position)
        .bind(title.clone().unwrap_or_else(|| file_title(name)))
        .bind(*duration)
        .bind(media)
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;
        file_ids.push(file_id);
    }
    if existing_book.is_some() {
        sqlx::query(
            "UPDATE audiobook_books SET total_duration_secs =
                 (SELECT sum(duration_secs) FROM audiobook_files WHERE book_id = $1), updated_at = now()
             WHERE id = $1",
        )
        .bind(book_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    for ((rel, _, size, mtime), id) in new_files.iter().zip(&file_ids) {
        remember(pool, folder.id, rel, *size, *mtime, "book_file", Some(*id)).await?;
    }
    if let Err(e) = crate::filesync::paths::ensure_book(pool, book_id).await {
        tracing::warn!(book = %book_id, error = %format!("{e:#}"), "library scan: no folder path for book");
    }
    Ok(if existing_book.is_some() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["10.mp3", "2.mp3", "1.mp3", "Chapter 11", "Chapter 9"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["1.mp3", "2.mp3", "10.mp3", "Chapter 9", "Chapter 11"]);
    }

    #[test]
    fn audio_and_titles() {
        assert!(is_audio("Track.MP3"));
        assert!(is_audio("book.m4b"));
        assert!(!is_audio("cover.jpg"));
        assert!(!is_audio("README"));
        assert_eq!(file_title("03_The_Ride.mp3"), "03 The Ride");
        assert_eq!(audio_content_type("a.m4b"), "audio/mp4");
    }

    #[test]
    fn configuration() {
        let mut cfg: crate::app::AppConfig = serde_json::from_value(serde_json::json!({
            "server": {}, "database_url": "x", "storage": {}, "auth": {"session_secret": "s"},
        }))
        .unwrap();
        assert!(configured(&cfg).unwrap().is_empty());
        cfg.library = Some(crate::app::LibraryConfig {
            music: Some("/music".into()),
            audiobooks: Some("/audiobooks".into()),
            folders: Some(r#"[{"path": "/more", "kind": "music", "visibility": "private"}]"#.into()),
            ..Default::default()
        });
        let got = configured(&cfg).unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], ("/more".to_string(), "music".to_string(), "private".to_string()));
        cfg.library.as_mut().unwrap().folders = Some(r#"[{"path": "/x", "kind": "video"}]"#.into());
        assert!(configured(&cfg).is_err());
    }
}
