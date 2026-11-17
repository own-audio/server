// SPDX-License-Identifier: AGPL-3.0-or-later
//! The rest of a track (docs/api-v2-completion-plan.md, phase 6): its lyrics written by
//! hand, what its file says about itself, reading that again, its cover, and the owner's
//! copies of one file.
//!
//! Who may: the track's owner (v1's rule); a track the caller cannot see is `404`, one they
//! see but do not own is `403`. A cover from an upload also needs the right to add to the
//! library. Everything else here is v1's code, called as it is.

use crate::app::AppState;
use crate::db::{self, catalog};
use crate::families::FamilyContext;
use crate::v2::discover::{Lyrics, answer as lyrics_answer};
use crate::v2::music::{Included, Track, tracks};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Lyrics typed or pasted: about a long song's worth, many times over.
const MAX_LYRICS_CHARS: usize = 100_000;
const MAX_LINES: usize = 5_000;

/// The track as the caller sees it (`404` otherwise) and theirs (`403` otherwise).
async fn owned(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<catalog::TrackRow, Problem> {
    let row = catalog::find_track(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    if row.user_id != family.user_id {
        return Err(Problem::forbidden().detail("only its owner changes a track"));
    }
    Ok(row)
}

async fn record(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<Track, Problem> {
    let row = catalog::find_track(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    tracks(state, vec![row]).await?.pop().ok_or_else(Problem::not_found)
}

// ── Lyrics ───────────────────────────────────────────────────────────────────

/// One timed line to write.
#[derive(Deserialize, ToSchema)]
pub struct LineInput {
    /// Where it starts; leave it out on every line for words without timing.
    pub start_ms: Option<i64>,
    pub text: String,
}

#[derive(Deserialize, ToSchema)]
pub struct LyricsRequest {
    /// The words as plain text, or as LRC (`[mm:ss.xx] line`) with their times. Empty or
    /// `null` takes the owner's lyrics away, and the file's own are read again.
    pub text: Option<String>,
    /// Or the lines (the shape `GET` answers), instead of `text`; kept as LRC when they
    /// carry times.
    pub lines: Option<Vec<LineInput>>,
}

/// `[mm:ss.xx]`, as LRC writes a time.
fn lrc_time(ms: i64) -> String {
    let centis = ms / 10;
    format!("[{:02}:{:02}.{:02}]", centis / 6000, (centis / 100) % 60, centis % 100)
}

/// The text to store, `None` to take the lyrics away.
fn lyrics_text(body: LyricsRequest) -> Result<Option<String>, Problem> {
    let text = match (body.text, body.lines) {
        (Some(_), Some(_)) => {
            return Err(Problem::bad_request("invalid_body", "send `text` or `lines`, not both").field("/lines", "invalid", "or `text`"));
        }
        (text, None) => text.unwrap_or_default(),
        (None, Some(lines)) => {
            if lines.len() > MAX_LINES {
                return Err(Problem::bad_request("invalid_body", format!("at most {MAX_LINES} lines")).field("/lines", "too_long", ""));
            }
            let timed = lines.iter().filter(|l| l.start_ms.is_some()).count();
            if timed != 0 && timed != lines.len() {
                return Err(Problem::bad_request("invalid_body", "every line has a time, or none does").field("/lines", "invalid", "times on some lines only"));
            }
            let mut out = String::new();
            let mut last = -1;
            for (i, line) in lines.iter().enumerate() {
                if line.text.contains('\n') {
                    return Err(Problem::bad_request("invalid_body", "a line is one line").field(format!("/lines/{i}/text"), "invalid", "no line breaks"));
                }
                if let Some(ms) = line.start_ms {
                    if ms < 0 || ms < last {
                        return Err(Problem::bad_request("invalid_body", "times start at 0 and do not go back")
                            .field(format!("/lines/{i}/start_ms"), "invalid", "0 or more, in order"));
                    }
                    last = ms;
                    out.push_str(&lrc_time(ms));
                }
                out.push_str(line.text.trim_end());
                out.push('\n');
            }
            out
        }
    };
    let text = text.trim();
    if text.chars().count() > MAX_LYRICS_CHARS {
        return Err(Problem::bad_request("invalid_body", format!("lyrics are at most {MAX_LYRICS_CHARS} characters")).field("/text", "too_long", ""));
    }
    Ok((!text.is_empty()).then(|| text.to_string()))
}

/// Write a track's lyrics by hand. They are kept as written (LRC times included) and win
/// over the file's own; taking them away reads the file again.
#[utoipa::path(put, path = "/tracks/{id}/lyrics", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = LyricsRequest,
    responses((status = 200, description = "The lyrics now", body = Lyrics),
        (status = 400, description = "`invalid_body`: both `text` and `lines`, lines with times on some only or going back, too long", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn put_lyrics(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<LyricsRequest>) -> Result<Json<Lyrics>, Problem> {
    owned(&state, &family, id).await?;
    match lyrics_text(body)? {
        Some(text) => {
            db::music::set_user_track_lyrics(state.db(), id, family.user_id, &text).await?;
            Ok(Json(lyrics_answer(text)))
        }
        None => {
            db::music::clear_user_track_lyrics(state.db(), id, family.user_id).await?;
            Ok(Json(lyrics_answer(String::new())))
        }
    }
}

// ── The file's own tags ──────────────────────────────────────────────────────

/// What a track's file says about itself.
#[derive(Serialize, ToSchema)]
pub struct FileTags {
    /// The file's own name, often the last honest clue when tags are missing or wrong.
    pub file_name: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub is_compilation: bool,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub disc_total: Option<i32>,
    /// Whether the file carries a picture.
    pub has_cover: bool,
}

/// The tags in a track's file, read now and written nowhere: what the file says, which may
/// differ from the library after an identify or an edit.
#[utoipa::path(get, path = "/tracks/{id}/file-tags", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = FileTags),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn file_tags(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<FileTags>, Problem> {
    owned(&state, &family, id).await?;
    let track = db::music::find_track_owned(state.db(), id, family.user_id).await?.ok_or_else(Problem::not_found)?;
    let (file_name, tags) = crate::music::file_tags(&state, track.audio_object_id).await?;
    Ok(Json(FileTags {
        file_name,
        title: tags.title,
        artist: tags.artist,
        album: tags.album,
        album_artist: tags.album_artist,
        is_compilation: tags.is_compilation,
        genre: tags.genre,
        track_number: tags.track_number,
        disc_number: tags.disc_number,
        disc_total: tags.disc_total,
        has_cover: tags.cover.is_some(),
    }))
}

/// Read a track's tags from its file again, into the library: the file wins where it has a
/// value, what is stored stays where it has none, and its picture becomes the cover. Not on
/// a track identified here (MusicBrainz is the better source).
#[utoipa::path(post, path = "/tracks/{id}/rescan", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "The track now", body = Track),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`identified`: identified through MusicBrainz; undo that first (`DELETE /tracks/{id}/identity`)", body = crate::v2::problem::ProblemBody),
        (status = 422, description = "`no_tags`: the file carries none", body = crate::v2::problem::ProblemBody)))]
pub async fn rescan(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Track>, Problem> {
    owned(&state, &family, id).await?;
    let track = db::music::find_track_owned(state.db(), id, family.user_id).await?.ok_or_else(Problem::not_found)?;
    match crate::music::rescan_track(&state, &family, &track).await? {
        crate::music::Rescan::Done => Ok(Json(record(&state, &family, id).await?)),
        crate::music::Rescan::Identified => Err(Problem::new(StatusCode::CONFLICT, "identified")
            .detail("identified through MusicBrainz: reading the file again would overwrite that; undo the identify first")),
        crate::music::Rescan::NoTags => Err(Problem::new(StatusCode::UNPROCESSABLE_ENTITY, "no_tags").detail("the file carries no tags to read")),
    }
}

// ── Cover ────────────────────────────────────────────────────────────────────

/// Put a cover on a track, from an upload (`POST /uploads`, kind `track_cover`). One change in
/// the edit history, which undoes it.
#[utoipa::path(put, path = "/tracks/{id}/image", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = crate::v2::books::ImageUpload,
    responses((status = 200, description = "The track now", body = Track),
        (status = 400, description = "`not_an_image`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the track's owner, or not allowed to add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible, or no such upload in this household", body = crate::v2::problem::ProblemBody),
        (status = 413, description = "`too_large`: over 20 MB", body = crate::v2::problem::ProblemBody)))]
pub async fn put_image(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<crate::v2::books::ImageUpload>) -> Result<Json<Track>, Problem> {
    owned(&state, &family, id).await?;
    family.require_can_upload()?;
    let image = crate::v2::books::image_of(&state, &family, &body).await?;
    db::music::set_track_cover(state.db(), id, family.user_id, Some(image), &db::edits::Edit::new(family.user_id, "manual")).await?;
    Ok(Json(record(&state, &family, id).await?))
}

/// Take a track's cover away (an edit, undone like any other).
#[utoipa::path(delete, path = "/tracks/{id}/image", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Removed, or there was none"),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_image(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    let row = owned(&state, &family, id).await?;
    if row.cover_object_id.is_some() {
        db::music::set_track_cover(state.db(), id, family.user_id, None, &db::edits::Edit::new(family.user_id, "manual")).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Duplicates ───────────────────────────────────────────────────────────────

/// Tracks of the caller's that are the same file.
#[derive(Serialize, ToSchema)]
pub struct DuplicateGroup {
    /// What they share: the file's SHA-256.
    pub key: String,
    /// `identical`: byte for byte the same file.
    pub kind: String,
    /// Oldest first, so the first is the one to keep when in doubt.
    pub track_ids: Vec<Uuid>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DuplicatesQuery {
    /// `tracks` embeds the tracks in `included`.
    pub include: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// The caller's own tracks that are the same file uploaded more than once, a group a file.
/// Found by the checksum the server keeps, so a song uploaded a moment ago joins once its
/// file has been read (`Track.sha256`). Another member's copy is theirs, never a duplicate.
#[utoipa::path(get, path = "/tracks/duplicates", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(DuplicatesQuery),
    responses((status = 200, body = Page<DuplicateGroup>),
        (status = 400, description = "`invalid_include`, `invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody)))]
pub async fn duplicates(family: FamilyContext, State(state): State<AppState>, Query(q): Query<DuplicatesQuery>) -> Result<Json<Page<DuplicateGroup>>, Problem> {
    let with_tracks = match q.include.as_deref().map(str::trim) {
        None | Some("") => false,
        Some("tracks") => true,
        Some(other) => {
            return Err(Problem::bad_request("invalid_include", format!("`{other}` cannot be included here; one of: tracks")).field("include", "invalid", other.to_string()));
        }
    };
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let after: Option<String> = page.after()?;
    let rows = catalog::duplicate_groups(state.db(), family.user_id, after.as_deref(), limit + 1).await?;
    let page = Page::from_rows(rows, limit, |(key, _)| key.clone());
    let included = if with_tracks {
        let ids: Vec<Uuid> = page.items.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
        let rows = catalog::tracks_by_ids(state.db(), family.viewer(), &ids).await?;
        Some(Included { tracks: tracks(&state, rows).await?, ..Default::default() })
    } else {
        None
    };
    let items = page.items.into_iter().map(|(key, track_ids)| DuplicateGroup { key, kind: "identical".into(), track_ids }).collect();
    Ok(Json(Page { items, next: page.next, included }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[(Option<i64>, &str)]) -> LyricsRequest {
        LyricsRequest { text: None, lines: Some(v.iter().map(|(ms, t)| LineInput { start_ms: *ms, text: t.to_string() }).collect()) }
    }

    #[test]
    fn timed_lines_become_lrc_and_read_back() {
        let text = lyrics_text(lines(&[(Some(1500), "One"), (Some(63_250), "Two")])).unwrap().unwrap();
        assert_eq!(text, "[00:01.50]One\n[01:03.25]Two");
        let back = lyrics_answer(text);
        assert!(back.synced);
        assert_eq!(back.lines.iter().map(|l| l.start_ms).collect::<Vec<_>>(), vec![Some(1500), Some(63_250)]);
        assert_eq!(lyrics_text(lines(&[(None, "a"), (None, "b")])).unwrap().unwrap(), "a\nb");
    }

    #[test]
    fn nothing_takes_them_away() {
        assert_eq!(lyrics_text(LyricsRequest { text: None, lines: None }).unwrap(), None);
        assert_eq!(lyrics_text(LyricsRequest { text: Some("  \n ".into()), lines: None }).unwrap(), None);
        assert_eq!(lyrics_text(lines(&[])).unwrap(), None);
    }

    #[test]
    fn bad_lines_are_refused() {
        let code = |r: LyricsRequest| lyrics_text(r).unwrap_err().code().to_string();
        assert_eq!(code(lines(&[(Some(10), "a"), (None, "b")])), "invalid_body");
        assert_eq!(code(lines(&[(Some(10), "a"), (Some(5), "b")])), "invalid_body");
        assert_eq!(code(lines(&[(Some(-1), "a")])), "invalid_body");
        assert_eq!(code(lines(&[(None, "a\nb")])), "invalid_body");
        assert_eq!(code(LyricsRequest { text: Some("x".into()), lines: Some(Vec::new()) }), "invalid_body");
        assert_eq!(code(LyricsRequest { text: Some("x".repeat(MAX_LYRICS_CHARS + 1)), lines: None }), "invalid_body");
    }
}
