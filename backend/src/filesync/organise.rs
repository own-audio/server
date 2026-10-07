// SPDX-License-Identifier: AGPL-3.0-or-later
/// "Organise" (file-sync-plan §2 item 18): move the owner's own tracks to
/// `Music/<Album artist>/<Album>/<NN> - <Title>` and books to
/// `Audiobooks/<Author>/<Title>`, only when asked and after a preview.
///
/// Preview and apply run the same code in one transaction; a preview rolls it
/// back, so the moves it lists — suffixes for collisions included — are the
/// ones apply would make. Moving items first go to a temporary path each, so
/// one taking another's old place never collides with it. Companion files go
/// with their book, and with an album's tracks when the whole folder moves to
/// one place. The sync feed carries the moves: identifiers do not change.
use super::paths::{self, Kind};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = OrganiseKind)]
pub enum What {
    Music,
    Audiobook,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = OrganiseRequest)]
pub struct Request {
    pub kind: What,
    /// True: list what would move, change nothing.
    #[serde(default)]
    pub preview: bool,
    /// Apply only these items; all of them when absent.
    #[serde(default)]
    pub ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize, PartialEq, utoipa::ToSchema)]
#[schema(as = OrganiseMove)]
pub struct Move {
    /// `music_track` or `audiobook`.
    pub kind: &'static str,
    pub id: Uuid,
    pub title: String,
    pub from: String,
    pub to: String,
    /// Companion files that move along.
    pub companions: usize,
}

struct Candidate {
    id: Uuid,
    title: String,
    from: String,
    wanted: String,
}

pub async fn organise(pool: &PgPool, owner: Uuid, request: &Request) -> anyhow::Result<Vec<Move>> {
    let mut tx = pool.begin().await.context("db: begin organise")?;
    // The same lock path claims take, so an upload cannot take a path mid-way.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('sync_paths:' || $1::text))")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .context("db: lock owner paths")?;

    let (kind, candidates) = match request.kind {
        What::Music => (Kind::MusicTrack, tracks(&mut tx, owner).await?),
        What::Audiobook => (Kind::Audiobook, books(&mut tx, owner).await?),
    };
    let only: Option<HashSet<Uuid>> = request.ids.as_ref().map(|ids| ids.iter().copied().collect());
    let moving: Vec<Candidate> = candidates
        .into_iter()
        .filter(|c| c.from.to_lowercase() != c.wanted.to_lowercase())
        .filter(|c| only.as_ref().is_none_or(|o| o.contains(&c.id)))
        .collect();

    for c in &moving {
        set_path(&mut tx, kind, c.id, &format!(".organising/{}", c.id)).await?;
    }
    let mut moves = Vec::new();
    for c in &moving {
        let to = paths::free_path(&mut tx, owner, kind, c.id, &c.wanted).await?;
        set_path(&mut tx, kind, c.id, &to).await?;
        if to != c.from {
            moves.push(Move { kind: kind.as_str(), id: c.id, title: c.title.clone(), from: c.from.clone(), to, companions: 0 });
        } else {
            set_path(&mut tx, kind, c.id, &c.from).await?;
        }
    }
    move_companions(&mut tx, owner, kind, &mut moves).await?;

    if request.preview {
        tx.rollback().await.context("db: roll back organise preview")?;
    } else {
        tx.commit().await.context("db: commit organise")?;
    }
    moves.sort_by_key(|a| a.to.to_lowercase());
    Ok(moves)
}

async fn tracks(tx: &mut Transaction<'_, Postgres>, owner: Uuid) -> anyhow::Result<Vec<Candidate>> {
    type Row = (Uuid, String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<i32>, Option<i32>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT t.id, t.title, t.album, t.album_artist, t.artist, t.track_number, t.disc_number, t.disc_total, p.path
           FROM music_tracks t JOIN sync_paths p ON p.kind = 'music_track' AND p.item_id = t.id
          WHERE t.user_id = $1",
    )
    .bind(owner)
    .fetch_all(&mut **tx)
    .await
    .context("db: tracks to organise")?;
    let album_of = |album_artist: &Option<String>, artist: &Option<String>, album: &Option<String>| {
        let artist = album_artist.clone().filter(|a| !a.trim().is_empty()).or_else(|| artist.clone());
        (artist.unwrap_or_default().to_lowercase(), album.clone().unwrap_or_default().to_lowercase())
    };
    // An album is multi-disc when any of its tracks says so; then every track with a disc
    // number goes into its `CD <n>` folder.
    let mut multi_disc: HashSet<(String, String)> = HashSet::new();
    for (_, _, album, album_artist, artist, _, disc, total, _) in &rows {
        if disc.is_some_and(|d| d > 1) || total.is_some_and(|t| t > 1) {
            multi_disc.insert(album_of(album_artist, artist, album));
        }
    }
    Ok(rows
        .into_iter()
        .map(|(id, title, album, album_artist, artist, track_number, disc, _, from)| {
            let disc = disc.filter(|_| multi_disc.contains(&album_of(&album_artist, &artist, &album)));
            let album_artist = album_artist.filter(|a| !a.trim().is_empty()).or(artist);
            // The file keeps its own extension; only the place and the name change.
            let ext = paths::file_extension(&from);
            let wanted = paths::default_track(album_artist.as_deref(), album.as_deref(), disc, track_number, &title, &ext);
            Candidate { id, title, from, wanted }
        })
        .collect())
}

async fn books(tx: &mut Transaction<'_, Postgres>, owner: Uuid) -> anyhow::Result<Vec<Candidate>> {
    let rows: Vec<(Uuid, String, Option<String>, String)> = sqlx::query_as(
        "SELECT b.id, b.title, b.author, p.path
           FROM audiobook_books b JOIN sync_paths p ON p.kind = 'audiobook' AND p.item_id = b.id
          WHERE b.user_id = $1",
    )
    .bind(owner)
    .fetch_all(&mut **tx)
    .await
    .context("db: books to organise")?;
    Ok(rows
        .into_iter()
        .map(|(id, title, author, from)| {
            let wanted = paths::default_book(author.as_deref(), &title);
            Candidate { id, title, from, wanted }
        })
        .collect())
}

async fn set_path(tx: &mut Transaction<'_, Postgres>, kind: Kind, id: Uuid, path: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_paths SET path = $3 WHERE kind = $1 AND item_id = $2")
        .bind(kind.as_str())
        .bind(id)
        .bind(path)
        .execute(&mut **tx)
        .await
        .context("db: move path")?;
    Ok(())
}

/// A book's companions move with its folder. An album folder's companions move when every
/// track that was directly in it moved, all to one new folder.
async fn move_companions(tx: &mut Transaction<'_, Postgres>, owner: Uuid, kind: Kind, moves: &mut [Move]) -> anyhow::Result<()> {
    if moves.is_empty() {
        return Ok(());
    }
    let companions: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT item_id, path FROM sync_live_paths WHERE user_id = $1 AND kind = 'companion_file'",
    )
    .bind(owner)
    .fetch_all(&mut **tx)
    .await
    .context("db: companions to organise")?;

    // Old folder → (new folder, index of the move that answers for it).
    let mut folders: HashMap<String, (String, usize)> = HashMap::new();
    match kind {
        Kind::Audiobook => {
            for (i, m) in moves.iter().enumerate() {
                folders.insert(m.from.to_lowercase(), (m.to.clone(), i));
            }
        }
        _ => {
            // A folder's companions follow when every track under it — CD subfolders
            // included — moved, and all to one folder. `Music` itself is no album.
            let staying: Vec<String> = sqlx::query_scalar(
                "SELECT lower(path) FROM sync_live_paths WHERE user_id = $1 AND kind = 'music_track'",
            )
            .bind(owner)
            .fetch_all(&mut **tx)
            .await
            .context("db: tracks after organise")?;
            let dirs: HashSet<String> = companions
                .iter()
                .map(|(_, p)| paths::parent_dir(p).to_lowercase())
                .filter(|d| d.contains('/'))
                .collect();
            for old in dirs {
                let prefix = format!("{old}/");
                let ixs: Vec<usize> =
                    (0..moves.len()).filter(|&i| moves[i].from.to_lowercase().starts_with(&prefix)).collect();
                if ixs.is_empty() {
                    continue;
                }
                // Paths are already the new ones: a track still under the old folder stayed.
                let someone_stays = staying.iter().any(|p| p.starts_with(&prefix));
                let targets: HashSet<String> = ixs.iter().map(|&i| paths::parent_dir(&moves[i].to).to_string()).collect();
                if targets.len() == 1 && !someone_stays {
                    let new = targets.into_iter().next().expect("one target");
                    folders.insert(old, (new, ixs[0]));
                }
            }
        }
    }

    for (id, path) in companions {
        let lower = path.to_lowercase();
        // The nearest folder wins: a cover in `CD 01` goes with `CD 01`, not with the album.
        let hit = folders.iter().filter(|(old, _)| lower.starts_with(&format!("{old}/"))).max_by_key(|(old, _)| old.len());
        let Some((old, (new, i))) = hit else { continue };
        let wanted = format!("{new}{}", &path[old.len()..]);
        let to = paths::free_path(tx, owner, Kind::CompanionFile, id, &wanted).await?;
        set_path(tx, Kind::CompanionFile, id, &to).await?;
        moves[*i].companions += 1;
    }
    Ok(())
}
