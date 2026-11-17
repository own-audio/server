// SPDX-License-Identifier: AGPL-3.0-or-later
//! The own.audio folder on `/api/v2` (completion plan phase 11): which of the household's
//! things a listener's folder shows (shortcuts), which of their devices keep what offline
//! (holdings), and moving their own items to the folder's default places (organise).
//!
//! The rules are v1's (`crate::filesync`), called rather than copied. What v2 changes: an
//! album is named by its id rather than by its MusicBrainz release group or its name; the
//! kinds are v2's words (`track`, `book`, `episode`, `file`); a device's holdings come a page
//! at a time; shortcuts reach the listener's other devices through the change feed
//! (`shortcut`).

use crate::app::AppState;
use crate::auth::middleware::AuthUser;
use crate::families::FamilyContext;
use crate::filesync::{holdings, organise, paths, shortcuts};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ── Shortcuts ────────────────────────────────────────────────────────────────

/// A thing of another member's that the listener's own.audio folder shows under `Family/`:
/// all of their music, books or podcasts, or one album, book or show.
#[derive(Serialize, ToSchema)]
pub struct Shortcut {
    pub id: Uuid,
    /// Whose (a `member` record).
    pub member_id: Uuid,
    /// `music`, `books` or `podcasts`.
    pub media: String,
    /// One album, book or show; none of the three for all of the member's `media`.
    pub album_id: Option<Uuid>,
    pub book_id: Option<Uuid>,
    pub podcast_id: Option<Uuid>,
    /// The album's, book's or show's title.
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct NewShortcut {
    /// Whose; found from the album, book or show when one is named.
    pub member_id: Option<Uuid>,
    /// `music`, `books` or `podcasts`; follows from the album, book or show when one is named.
    pub media: Option<String>,
    /// At most one of these three.
    pub album_id: Option<Uuid>,
    pub book_id: Option<Uuid>,
    pub podcast_id: Option<Uuid>,
}

pub(crate) fn media_word(v1: &str) -> &'static str {
    match v1 {
        "audiobook" => "books",
        "podcast" => "podcasts",
        _ => "music",
    }
}

pub(crate) fn shortcut(row: shortcuts::ShortcutRow) -> Shortcut {
    let (album_id, book_id, podcast_id) = match row.container_kind.as_deref() {
        Some("album") => (row.album_id, None, None),
        Some("book") => (None, row.container_id, None),
        Some("show") => (None, None, row.container_id),
        _ => (None, None, None),
    };
    Shortcut { id: row.id, member_id: row.member_id, media: media_word(&row.kind).to_string(), album_id, book_id, podcast_id, label: row.label, created_at: row.created_at }
}

/// The listener's shortcuts, oldest first. Bounded by the household, so not paged.
#[utoipa::path(get, path = "/sync/shortcuts", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Vec<Shortcut>)))]
pub async fn list_shortcuts(family: FamilyContext, State(state): State<AppState>) -> Result<Json<Vec<Shortcut>>, Problem> {
    let rows = shortcuts::rows(state.db(), family.user_id, None).await?;
    Ok(Json(rows.into_iter().map(shortcut).collect()))
}

/// v1's media kind, and the container's v1 kind and id when one is named.
pub(crate) type V1Shortcut = (Option<&'static str>, Option<(&'static str, Uuid)>);

/// The request as v1 takes it: a media kind, and a container when one is named.
pub(crate) fn v1_request(body: &NewShortcut) -> Result<V1Shortcut, Problem> {
    let named: Vec<(&'static str, Uuid)> = [("album", body.album_id), ("book", body.book_id), ("show", body.podcast_id)]
        .into_iter()
        .filter_map(|(k, id)| id.map(|id| (k, id)))
        .collect();
    if named.len() > 1 {
        return Err(Problem::bad_request("invalid_body", "name one of album_id, book_id and podcast_id, or none").field("/album_id", "conflict", ""));
    }
    let media = match body.media.as_deref().map(str::trim) {
        None | Some("") => None,
        Some("music") => Some("music"),
        Some("books") => Some("audiobook"),
        Some("podcasts") => Some("podcast"),
        Some(other) => return Err(Problem::bad_request("invalid_media", format!("`{other}`: media is `music`, `books` or `podcasts`")).field("/media", "invalid", other.to_string())),
    };
    let container = named.first().copied();
    let implied = container.map(|(k, _)| match k {
        "album" => "music",
        "book" => "audiobook",
        _ => "podcast",
    });
    match (media, implied) {
        (Some(m), Some(i)) if m != i => Err(Problem::bad_request("invalid_media", "the media does not match the album, book or show").field("/media", "conflict", "")),
        (None, None) => Err(Problem::bad_request("invalid_body", "name the media, or an album, book or show").field("/media", "required", "")),
        (m, i) => Ok((m.or(i), container)),
    }
}

/// Show another member's music, books or podcasts in the listener's folder, or one album, book
/// or show of theirs. `201` when new, `200` with the one already there.
#[utoipa::path(post, path = "/sync/shortcuts", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = NewShortcut,
    responses((status = 201, body = Shortcut, description = "Added"), (status = 200, body = Shortcut, description = "Already there"),
        (status = 400, description = "`invalid_body` (one's own, nothing named, two things named), `invalid_media`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "No such member in the household, or no such album, book or show of theirs the caller can see", body = crate::v2::problem::ProblemBody)))]
pub async fn add_shortcut(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewShortcut>) -> Result<(StatusCode, Json<Shortcut>), Problem> {
    let (media, container) = v1_request(&body)?;
    let media = media.unwrap_or("music");
    let viewer = family.viewer();
    let mut request = shortcuts::NewShortcut { member_id: body.member_id, kind: media.to_string(), container: None };
    if let Some((kind, id)) = container {
        let mut c = shortcuts::Container { kind: kind.to_string(), id: None, release_group: None, album_artist: None, album: None };
        if kind == "album" {
            // v1 keeps an album by its release group or its name, which its sync clients match on.
            let Some((owner, group, artist, album)) = shortcuts::album_identity(state.db(), viewer, id, body.member_id).await? else {
                return Err(Problem::not_found().detail("no album of another member's that you can see"));
            };
            request.member_id = Some(owner);
            c.release_group = group;
            c.album_artist = artist;
            c.album = album;
        } else {
            c.id = Some(id);
        }
        request.container = Some(c);
    }
    let (row, created) = match shortcuts::add(state.db(), viewer, request).await? {
        Ok((s, created)) => (s.id, created),
        Err(shortcuts::Refusal::Invalid(why)) => return Err(Problem::bad_request("invalid_body", why)),
        Err(shortcuts::Refusal::NotFound) => return Err(Problem::not_found()),
    };
    let found = shortcuts::rows(state.db(), family.user_id, Some(&[row])).await?.pop().ok_or_else(Problem::not_found)?;
    Ok((if created { StatusCode::CREATED } else { StatusCode::OK }, Json(shortcut(found))))
}

/// Stop showing it in the listener's folder.
#[utoipa::path(delete, path = "/sync/shortcuts/{id}", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not one of the caller's shortcuts", body = crate::v2::problem::ProblemBody)))]
pub async fn remove_shortcut(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    if shortcuts::remove(state.db(), family.user_id, id).await? { Ok(StatusCode::NO_CONTENT) } else { Err(Problem::not_found()) }
}

// ── Holdings ─────────────────────────────────────────────────────────────────

/// The v1 kind a holding is stored under, from v2's word.
pub(crate) fn holding_kind(word: &str) -> Option<paths::Kind> {
    match word {
        "book" => Some(paths::Kind::Audiobook),
        "track" => Some(paths::Kind::MusicTrack),
        "episode" => Some(paths::Kind::PodcastEpisode),
        "file" => Some(paths::Kind::CompanionFile),
        _ => None,
    }
}

pub(crate) fn holding_word(kind: &str) -> &'static str {
    match paths::Kind::parse(kind) {
        Some(paths::Kind::Audiobook) => "book",
        Some(paths::Kind::PodcastEpisode) => "episode",
        Some(paths::Kind::CompanionFile) => "file",
        _ => "track",
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct HoldingRef {
    /// `track`, `book`, `episode` or `file`.
    pub kind: String,
    pub id: Uuid,
}

#[derive(Serialize, ToSchema)]
pub struct Holding {
    pub kind: String,
    pub id: Uuid,
    /// Since when this device keeps it.
    pub since: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct HoldingsChange {
    /// The device's whole set, replacing what it said before…
    pub items: Option<Vec<HoldingRef>>,
    /// …or what it took in…
    #[serde(default)]
    pub added: Vec<HoldingRef>,
    /// …and let go.
    #[serde(default)]
    pub removed: Vec<HoldingRef>,
}

fn v1_keys(refs: &[HoldingRef], field: &str) -> Result<Vec<holdings::HoldingKey>, Problem> {
    refs.iter()
        .enumerate()
        .map(|(i, r)| match holding_kind(&r.kind) {
            Some(k) => Ok(holdings::HoldingKey { kind: k.as_str().to_string(), id: r.id }),
            None => Err(Problem::bad_request("invalid_kind", format!("`{}`: kind is `track`, `book`, `episode` or `file`", r.kind))
                .field(format!("/{field}/{i}/kind"), "invalid", r.kind.clone())),
        })
        .collect()
}

/// What the calling device keeps offline, by kind and id, a page at a time. The device is the
/// session's (`device_id` as in progress).
#[utoipa::path(get, path = "/me/holdings", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<Holding>)))]
pub async fn get_holdings(auth: AuthUser, _family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<Holding>>, Problem> {
    let limit = page.limit()?;
    let after = page.after::<(String, Uuid)>()?;
    let rows = holdings::held_page(state.db(), auth.user_id, crate::v2::state::device_id(&auth), after, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |h| (h.kind.clone(), h.id));
    Ok(Json(Page {
        items: page.items.into_iter().map(|h| Holding { kind: holding_word(&h.kind).to_string(), id: h.id, since: h.since }).collect(),
        next: page.next,
        included: None,
    }))
}

/// Tell the server what this device keeps: the whole set (`items`), or what it took in and
/// let go (`added`, `removed`).
#[utoipa::path(put, path = "/me/holdings", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = HoldingsChange,
    responses((status = 204, description = "Saved"), (status = 400, description = "`invalid_kind`", body = crate::v2::problem::ProblemBody)))]
pub async fn put_holdings(auth: AuthUser, _family: FamilyContext, State(state): State<AppState>, Json(body): Json<HoldingsChange>) -> Result<StatusCode, Problem> {
    let update = holdings::HoldingsUpdate {
        items: body.items.as_deref().map(|i| v1_keys(i, "items")).transpose()?,
        added: v1_keys(&body.added, "added")?,
        removed: v1_keys(&body.removed, "removed")?,
    };
    match holdings::update(state.db(), auth.user_id, crate::v2::state::device_id(&auth), update).await? {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(why) => Err(Problem::bad_request("invalid_kind", why)),
    }
}

#[derive(Serialize, ToSchema)]
pub struct HoldingDevice {
    pub device_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: Option<String>,
    pub since: DateTime<Utc>,
    /// The device asking.
    pub current: bool,
}

/// Which of the caller's signed-in devices keep this item offline.
#[utoipa::path(get, path = "/me/holdings/{kind}/{id}", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("kind" = String, Path, description = "`track`, `book`, `episode` or `file`"), ("id" = Uuid, Path)),
    responses((status = 200, body = Vec<HoldingDevice>), (status = 400, description = "`invalid_kind`", body = crate::v2::problem::ProblemBody)))]
pub async fn holding_devices(auth: AuthUser, _family: FamilyContext, State(state): State<AppState>, Path((kind, id)): Path<(String, Uuid)>) -> Result<Json<Vec<HoldingDevice>>, Problem> {
    let k = holding_kind(&kind)
        .ok_or_else(|| Problem::bad_request("invalid_kind", "kind is `track`, `book`, `episode` or `file`").field("kind", "invalid", kind.clone()))?;
    let this = crate::v2::state::device_id(&auth);
    let devices = holdings::devices_holding(state.db(), auth.user_id, k.as_str(), id).await?;
    Ok(Json(devices.into_iter().map(|d| HoldingDevice { current: d.chain_id == this, device_id: d.chain_id, device_name: d.device_name, device_kind: d.device_kind, since: d.since }).collect()))
}

// ── Organise ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct OrganiseRequest {
    /// `track` or `book`: which of the caller's own items.
    pub kind: String,
    /// Required, so nothing moves by accident: `true` lists the moves and changes nothing.
    pub dry_run: bool,
    /// Only these; every one of the kind when absent.
    pub ids: Option<Vec<Uuid>>,
}

#[derive(Serialize, ToSchema)]
pub struct Move {
    /// `track` or `book`.
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    pub from: String,
    pub to: String,
    /// How many of the folder's files (covers, booklets) go along.
    pub files: usize,
}

#[derive(Serialize, ToSchema)]
pub struct Organised {
    pub dry_run: bool,
    /// By the place they go to. A dry run lists exactly the moves the same request without it
    /// makes (it runs them and rolls back), suffixes for clashes included.
    pub moves: Vec<Move>,
}

/// Move the caller's own tracks to `Music/<Album artist>/<Album>/<NN> - <Title>` and books to
/// `Audiobooks/<Author>/<Title>`, with the files beside them; ids stay, the change feed carries
/// the new paths. Only the caller's own items, whatever their role.
#[utoipa::path(post, path = "/library/organise", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = OrganiseRequest,
    responses((status = 200, body = Organised), (status = 400, description = "`invalid_kind`", body = crate::v2::problem::ProblemBody), (status = 422, description = "`invalid_body`: `dry_run` missing", body = crate::v2::problem::ProblemBody)))]
pub async fn organise_paths(family: FamilyContext, State(state): State<AppState>, Json(body): Json<OrganiseRequest>) -> Result<Json<Organised>, Problem> {
    let kind = match body.kind.as_str() {
        "track" => organise::What::Music,
        "book" => organise::What::Audiobook,
        other => return Err(Problem::bad_request("invalid_kind", format!("`{other}`: kind is `track` or `book`")).field("/kind", "invalid", other.to_string())),
    };
    let request = organise::Request { kind, preview: body.dry_run, ids: body.ids };
    let moves = organise::organise(state.db(), family.user_id, &request).await?;
    Ok(Json(Organised {
        dry_run: body.dry_run,
        moves: moves
            .into_iter()
            .map(|m| Move { kind: if m.kind == "audiobook" { "book" } else { "track" }.to_string(), id: m.id, title: m.title, from: m.from, to: m.to, files: m.companions })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(media: Option<&str>, album: bool, book: bool) -> NewShortcut {
        NewShortcut {
            member_id: None,
            media: media.map(str::to_string),
            album_id: album.then(Uuid::new_v4),
            book_id: book.then(Uuid::new_v4),
            podcast_id: None,
        }
    }

    #[test]
    fn a_shortcut_names_one_thing_of_one_media() {
        assert_eq!(v1_request(&req(Some("books"), false, false)).unwrap().0, Some("audiobook"));
        assert_eq!(v1_request(&req(None, true, false)).unwrap().0, Some("music"));
        assert_eq!(v1_request(&req(Some("music"), true, false)).unwrap().1.unwrap().0, "album");
        assert_eq!(v1_request(&req(Some("books"), true, false)).unwrap_err().code(), "invalid_media");
        assert_eq!(v1_request(&req(None, true, true)).unwrap_err().code(), "invalid_body");
        assert_eq!(v1_request(&req(None, false, false)).unwrap_err().code(), "invalid_body");
        assert_eq!(v1_request(&req(Some("films"), false, false)).unwrap_err().code(), "invalid_media");
    }

    #[test]
    fn holdings_speak_v2_words() {
        for w in ["track", "book", "episode", "file"] {
            assert_eq!(holding_word(holding_kind(w).unwrap().as_str()), w);
        }
        assert!(holding_kind("music_track").is_none(), "v1's words are not v2's");
        assert_eq!(media_word("audiobook"), "books");
    }
}
