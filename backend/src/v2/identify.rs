// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identify runs (`features.identify_library`): match a library, a folder, an
//! album, a playlist, an artist or a selection against MusicBrainz in the
//! background, then go through what it proposes — accept every sure match at
//! once, look at the rest one by one, pick another edition, keep a
//! compilation's own album — and undo a whole run if it went wrong. The
//! database changes; files never do.
//!
//! Against musicbrainz.org a run goes at about one request a second, shared
//! with every other run on the server: a large library takes hours, and the run
//! says so by its progress rather than by failing.

use crate::app::AppState;
use crate::db;
use crate::families::FamilyContext;
use crate::hooks::IdentifyVerdict;
use crate::metadata::{Identify, MetadataCandidate};
use crate::music::identify_runs::{self, AcceptOutcome, matching::{self, ProposedTrack}};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const SCOPES: &[&str] = &["library", "folder", "album", "playlist", "artist", "tracks"];

#[derive(Deserialize, ToSchema)]
pub struct Scope {
    /// `library`, `folder` (a library folder's id), `album`, `playlist`,
    /// `artist` (a contributor id) or `tracks` (with `track_ids`).
    pub kind: String,
    pub id: Option<Uuid>,
    pub track_ids: Option<Vec<Uuid>>,
    /// How the run is named in lists, e.g. the album's title.
    pub label: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct NewRun {
    pub scope: Scope,
}

#[derive(Serialize, ToSchema)]
pub struct RunScope {
    pub kind: String,
    pub id: Option<Uuid>,
    pub label: Option<String>,
}

#[derive(Serialize, ToSchema, Default)]
pub struct RunCounts {
    pub sure: i64,
    pub review: i64,
    pub none: i64,
    pub pending: i64,
    pub accepted: i64,
    pub rejected: i64,
}

#[derive(Serialize, ToSchema)]
pub struct Run {
    pub id: Uuid,
    /// `queued`, `running`, `waiting` (the source asked us to slow down; it
    /// resumes by itself), `ready`, `failed` or `cancelled`.
    pub status: String,
    pub error: Option<String>,
    pub scope: RunScope,
    /// Tracks in scope without a MusicBrainz identity when the run started.
    pub tracks: i32,
    pub groups_total: i32,
    pub groups_done: i32,
    pub counts: RunCounts,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

async fn run_out(state: &AppState, r: db::identify::Run) -> Result<Run, Problem> {
    let mut counts = RunCounts::default();
    for (class, decision, n) in db::identify::counts(state.db(), r.id).await? {
        match class.as_str() {
            "sure" => counts.sure += n,
            "review" => counts.review += n,
            _ => counts.none += n,
        }
        match decision.as_str() {
            "accepted" => counts.accepted += n,
            "rejected" => counts.rejected += n,
            _ => counts.pending += n,
        }
    }
    Ok(Run {
        id: r.id,
        status: r.status,
        error: r.error,
        scope: RunScope { kind: r.scope_kind, id: r.scope_id, label: r.scope_label },
        tracks: r.tracks,
        groups_total: r.groups_total,
        groups_done: r.groups_done,
        counts,
        created_at: r.created_at,
        started_at: r.started_at,
        finished_at: r.finished_at,
    })
}

fn source_configured(state: &AppState) -> Result<(), Problem> {
    let cfg = state.config();
    if cfg.metadata.is_some() || cfg.musicbrainz.enabled {
        Ok(())
    } else {
        Err(Problem::new(StatusCode::NOT_IMPLEMENTED, "feature_unavailable")
            .detail("no MusicBrainz source is configured on this server"))
    }
}

async fn own_run(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<db::identify::Run, Problem> {
    db::identify::find_run_owned(state.db(), id, family.user_id).await?.ok_or_else(Problem::not_found)
}

/// Start a run over your own tracks in a scope. It looks up only tracks that
/// have no MusicBrainz identity yet; proposals arrive as it goes.
#[utoipa::path(post, path = "/identify/runs", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    request_body = NewRun,
    responses((status = 202, description = "Queued", body = Run),
        (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 402, description = "Not enough credit for the quote", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "No MusicBrainz source configured", body = crate::v2::problem::ProblemBody)))]
pub async fn create_run(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewRun>) -> Result<(StatusCode, Json<Run>), Problem> {
    source_configured(&state)?;
    let s = body.scope;
    let needs_id = check_scope(&s)?;
    let run = db::identify::create_run(
        state.db(),
        family.user_id,
        Some(family.family_id),
        &s.kind,
        s.id.filter(|_| needs_id),
        s.track_ids.as_deref(),
        s.label.as_deref().map(str::trim).filter(|l| !l.is_empty()),
    )
    .await?;
    let tracks: Vec<Uuid> = db::identify::scope_tracks(state.db(), &run).await?.into_iter().map(|t| t.id).collect();
    let verdict = state.hooks().identify_run_start(state.db(), family.family_id, family.user_id, run.id, &tracks).await;
    match verdict {
        Ok(IdentifyVerdict::Allowed) => {}
        Ok(IdentifyVerdict::Refused(why)) => {
            db::identify::delete_run(state.db(), run.id).await?;
            return Err(Problem::new(StatusCode::PAYMENT_REQUIRED, "payment_required").detail(why));
        }
        Err(e) => {
            db::identify::delete_run(state.db(), run.id).await?;
            return Err(e.into());
        }
    }
    identify_runs::start(state.db(), run.id).await?;
    Ok((StatusCode::ACCEPTED, Json(run_out(&state, run).await?)))
}

#[derive(Deserialize, ToSchema)]
pub struct QuoteRequest {
    pub scope: Scope,
}

#[derive(Serialize, ToSchema)]
pub struct Quote {
    /// Tracks in the scope that a run would look up: those with no MusicBrainz
    /// identity yet.
    pub tracks: usize,
    /// The most the run could charge, in micro-currency; it charges only for
    /// tracks it finds a match for. 0 where identify is free.
    pub charge_micro: i64,
}

/// What a run over this scope would look up, and the most it could cost.
/// Ask before starting one.
#[utoipa::path(post, path = "/identify/quote", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    request_body = QuoteRequest,
    responses((status = 200, body = Quote),
        (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 501, description = "No MusicBrainz source configured", body = crate::v2::problem::ProblemBody)))]
pub async fn quote(family: FamilyContext, State(state): State<AppState>, Json(body): Json<QuoteRequest>) -> Result<Json<Quote>, Problem> {
    source_configured(&state)?;
    let s = body.scope;
    let needs_id = check_scope(&s)?;
    let tracks: Vec<Uuid> = db::identify::scope_tracks_of(state.db(), family.user_id, &s.kind, s.id.filter(|_| needs_id), s.track_ids.as_deref())
        .await?
        .into_iter()
        .map(|t| t.id)
        .collect();
    let charge_micro = state.hooks().identify_quote(state.db(), family.family_id, &tracks).await?;
    Ok(Json(Quote { tracks: tracks.len(), charge_micro }))
}

/// Whether the scope is well formed; true when its kind needs an id.
fn check_scope(s: &Scope) -> Result<bool, Problem> {
    if !SCOPES.contains(&s.kind.as_str()) {
        return Err(Problem::bad_request("invalid_scope", format!("scope.kind is one of: {}", SCOPES.join(", ")))
            .field("/scope/kind", "invalid", SCOPES.join(", ")));
    }
    let needs_id = !matches!(s.kind.as_str(), "library" | "tracks");
    if needs_id && s.id.is_none() {
        return Err(Problem::bad_request("invalid_scope", format!("a `{}` scope needs its id", s.kind)).field("/scope/id", "required", "an id"));
    }
    if s.kind == "tracks" && s.track_ids.as_ref().is_none_or(|t| t.is_empty()) {
        return Err(Problem::bad_request("invalid_scope", "a `tracks` scope needs track_ids").field("/scope/track_ids", "required", "track ids"));
    }
    Ok(needs_id)
}

/// Your runs, newest first.
#[utoipa::path(get, path = "/identify/runs", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    responses((status = 200, body = Page<Run>)))]
pub async fn list_runs(family: FamilyContext, State(state): State<AppState>) -> Result<Json<Page<Run>>, Problem> {
    let mut items = Vec::new();
    for r in db::identify::list_runs(state.db(), family.user_id, 50).await? {
        items.push(run_out(&state, r).await?);
    }
    Ok(Json(Page { items, next: None, included: None }))
}

/// A run's progress and counts.
#[utoipa::path(get, path = "/identify/runs/{id}", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Run), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn get_run(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Run>, Problem> {
    let run = own_run(&state, &family, id).await?;
    Ok(Json(run_out(&state, run).await?))
}

/// Stop a run and forget its proposals. What was accepted stays accepted
/// (revert first to undo it).
#[utoipa::path(delete, path = "/identify/runs/{id}", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Gone"), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn delete_run(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    own_run(&state, &family, id).await?;
    // Settle while the proposals still say what was found.
    state.hooks().identify_run_end(state.db(), id).await?;
    db::identify::set_status(state.db(), id, "cancelled", None).await?;
    db::identify::delete_run(state.db(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct Reverted {
    /// Tracks that went back to what they were before the run.
    pub reverted: usize,
}

/// Undo every accept of this run that still stands. Its proposals become
/// pending again.
#[utoipa::path(post, path = "/identify/runs/{id}/revert", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Reverted), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn revert_run(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Reverted>, Problem> {
    let run = own_run(&state, &family, id).await?;
    Ok(Json(Reverted { reverted: identify_runs::revert_run(state.db(), &run).await? }))
}

#[derive(Deserialize, ToSchema)]
pub struct AcceptAll {
    /// Only `sure`: the review class is there to be looked at.
    pub class: String,
}

#[derive(Serialize, ToSchema)]
pub struct Accepted {
    pub accepted: usize,
}

/// Accept every pending sure proposal of a run.
#[utoipa::path(post, path = "/identify/runs/{id}/accept", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = AcceptAll,
    responses((status = 200, body = Accepted), (status = 400, body = crate::v2::problem::ProblemBody), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn accept_all(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<AcceptAll>) -> Result<Json<Accepted>, Problem> {
    let run = own_run(&state, &family, id).await?;
    if body.class != "sure" {
        return Err(Problem::bad_request("invalid_class", "only sure proposals are accepted in bulk").field("/class", "invalid", "sure"));
    }
    let mut accepted = 0;
    for pid in db::identify::pending_ids(state.db(), run.id, "sure").await? {
        if identify_runs::accept(state.db(), family.user_id, pid).await? == AcceptOutcome::Accepted {
            accepted += 1;
        }
    }
    Ok(Json(Accepted { accepted }))
}

#[derive(Serialize, ToSchema)]
pub struct ProposalGroup {
    pub id: Uuid,
    /// `album` or `track`.
    pub kind: String,
    pub label: String,
}

#[derive(Serialize, ToSchema)]
pub struct ProposalTrack {
    pub id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub disc_number: Option<i32>,
    pub track_number: Option<i32>,
    pub duration_secs: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct Proposal {
    pub id: Uuid,
    pub group: ProposalGroup,
    /// The track as it is now.
    pub track: ProposalTrack,
    /// `sure`, `review` or `none`.
    pub class: String,
    /// Why it is not sure: `incomplete` (the files are part of the release),
    /// `partial` (some files are not on it), `layout` (discs or numbering differ
    /// from the files'), `duplicate` (two files on one track), `ambiguous`
    /// (another release matches as well), `album_differs` (a compilation's
    /// song on its original album), `weak`, `not_found`, `not_on_release`,
    /// `chosen` (the owner picked the recording).
    /// Unknown values may appear.
    pub reasons: Vec<String>,
    /// What an accept writes; absent when nothing was found.
    pub proposal: Option<ProposedTrack>,
    /// Accept sets only the recording and keeps the track's album.
    pub keep_album: bool,
    /// `pending`, `accepted` or `rejected`.
    pub decision: String,
    /// The owner picked this recording (`PUT …/recording`); another edition of
    /// the album leaves it as it is.
    pub chosen_by_owner: bool,
    /// Other recordings stored with it (`GET …/alternatives`).
    pub alternatives: i32,
}

fn proposal_out(p: db::identify::ProposalRow) -> Proposal {
    Proposal {
        id: p.id,
        group: ProposalGroup { id: p.group_id, kind: p.group_kind, label: p.group_label },
        track: ProposalTrack {
            id: p.track_id,
            title: p.title,
            artist: p.artist,
            album: p.album,
            disc_number: p.disc_number,
            track_number: p.track_number,
            duration_secs: p.duration_secs,
        },
        class: p.class,
        reasons: p.reasons,
        proposal: p.proposal.and_then(|v| serde_json::from_value(v).ok()),
        keep_album: p.keep_album,
        decision: p.decision,
        chosen_by_owner: p.chosen_by_owner,
        alternatives: p.alternatives_count,
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProposalFilter {
    /// `sure`, `review` or `none`.
    pub class: Option<String>,
    /// `pending`, `accepted` or `rejected`.
    pub decision: Option<String>,
}

/// A run's proposals, an album's tracks together.
#[utoipa::path(get, path = "/identify/runs/{id}/proposals", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path), ProposalFilter, PageQuery),
    responses((status = 200, body = Page<Proposal>), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn list_proposals(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(filter): Query<ProposalFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Proposal>>, Problem> {
    let run = own_run(&state, &family, id).await?;
    let limit = page.limit()?;
    let after: Option<(i32, Uuid)> = page.after()?;
    let rows = db::identify::list_proposals(state.db(), run.id, filter.class.as_deref(), filter.decision.as_deref(), after, limit + 1).await?;
    let keys: Vec<(i32, Uuid)> = rows.iter().map(|r| (r.group_position, r.id)).collect();
    let mut page = Page::from_rows(rows.into_iter().map(proposal_out).collect::<Vec<_>>(), limit, |p| p.id);
    // The cursor is (group position, id): rebuild it from the last row kept.
    page.next = page.next.as_ref().and_then(|_| {
        keys.get(page.items.len().saturating_sub(1)).map(crate::v2::page::encode)
    });
    Ok(Json(page))
}

async fn own_proposal(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<db::identify::ProposalRow, Problem> {
    let p = db::identify::find_proposal(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    own_run(state, family, p.run_id).await?;
    Ok(p)
}

/// Apply one proposal to its track.
#[utoipa::path(post, path = "/identify/proposals/{id}/accept", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Proposal),
        (status = 409, description = "Already decided, or nothing was found to apply", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn accept(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Proposal>, Problem> {
    own_proposal(&state, &family, id).await?;
    match identify_runs::accept(state.db(), family.user_id, id).await? {
        AcceptOutcome::Accepted => {}
        AcceptOutcome::AlreadyDecided => return Err(Problem::new(StatusCode::CONFLICT, "already_decided").detail("this proposal was already accepted or rejected")),
        AcceptOutcome::NothingProposed => return Err(Problem::new(StatusCode::CONFLICT, "nothing_proposed").detail("nothing was found for this track")),
    }
    Ok(Json(proposal_out(own_proposal(&state, &family, id).await?)))
}

/// Leave the track as it is.
#[utoipa::path(post, path = "/identify/proposals/{id}/reject", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Proposal), (status = 409, body = crate::v2::problem::ProblemBody), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn reject(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Proposal>, Problem> {
    let p = own_proposal(&state, &family, id).await?;
    if p.decision != "pending" {
        return Err(Problem::new(StatusCode::CONFLICT, "already_decided").detail("this proposal was already accepted or rejected"));
    }
    db::identify::set_decision(state.db(), id, "rejected").await?;
    Ok(Json(proposal_out(own_proposal(&state, &family, id).await?)))
}

#[derive(Deserialize, ToSchema)]
pub struct ProposalPatch {
    /// Accept only the recording and keep the track's own album.
    pub keep_album: bool,
}

/// Change how a pending proposal will be applied.
#[utoipa::path(patch, path = "/identify/proposals/{id}", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = ProposalPatch,
    responses((status = 200, body = Proposal), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn patch_proposal(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<ProposalPatch>) -> Result<Json<Proposal>, Problem> {
    own_proposal(&state, &family, id).await?;
    db::identify::set_keep_album(state.db(), id, body.keep_album).await?;
    Ok(Json(proposal_out(own_proposal(&state, &family, id).await?)))
}

#[derive(Serialize, ToSchema)]
pub struct Edition {
    pub index: usize,
    pub mb_release_id: String,
    pub mb_release_group_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub primary_type: Option<String>,
    pub secondary_types: Vec<String>,
    pub status: Option<String>,
    pub track_count: i32,
    /// How many of the group's tracks it holds.
    pub matched: u32,
    pub cover_art_url: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Group {
    pub id: Uuid,
    pub kind: String,
    pub label: String,
    /// Index into `editions` of the one the proposals are on.
    pub chosen: Option<i16>,
    pub editions: Vec<Edition>,
}

async fn group_out(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<Group, Problem> {
    let g = db::identify::find_group(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    own_run(state, family, g.run_id).await?;
    let candidates: Vec<crate::metadata::AlbumCandidate> = serde_json::from_value(g.candidates).unwrap_or_default();
    Ok(Group {
        id: g.id,
        kind: g.kind,
        label: g.label,
        chosen: g.chosen,
        editions: candidates
            .into_iter()
            .enumerate()
            .map(|(index, c)| Edition {
                index,
                mb_release_id: c.mb_release_id,
                mb_release_group_id: c.mb_release_group_id,
                title: c.title,
                artist: c.artist,
                year: c.year,
                primary_type: c.primary_type,
                secondary_types: c.secondary_types,
                status: c.status,
                track_count: c.track_count,
                matched: c.matched,
                cover_art_url: c.cover_art_url,
            })
            .collect(),
    })
}

/// An album group and the releases it could be, for picking another edition.
#[utoipa::path(get, path = "/identify/groups/{id}", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Group), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn get_group(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Group>, Problem> {
    Ok(Json(group_out(&state, &family, id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct EditionChoice {
    /// An index from the group's `editions`.
    pub index: usize,
}

/// Put an album group's pending proposals on another of its editions. No
/// lookup: the editions were fetched with the run.
#[utoipa::path(put, path = "/identify/groups/{id}/edition", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = EditionChoice,
    responses((status = 200, body = Group), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn choose_edition(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<EditionChoice>) -> Result<Json<Group>, Problem> {
    let g = db::identify::find_group(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    let run = own_run(&state, &family, g.run_id).await?;
    if !identify_runs::choose_edition(state.db(), &run, id, body.index).await? {
        return Err(Problem::not_found());
    }
    Ok(Json(group_out(&state, &family, id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct AlbumSearch {
    /// The album's title as the owner types it; the tags' by default.
    pub album: Option<String>,
    /// The album's artist as the owner types it; the tags' by default.
    pub artist: Option<String>,
}

/// Search an album's editions again with the owner's words, for an album whose
/// tags name it wrong, and put its pending proposals on the best one. Nothing
/// found leaves the group as it was (`editions` then are the old ones).
#[utoipa::path(post, path = "/identify/groups/{id}/search", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = AlbumSearch,
    responses((status = 200, body = Group), (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown, or not an album group", body = crate::v2::problem::ProblemBody)))]
pub async fn search_group(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<AlbumSearch>) -> Result<Json<Group>, Problem> {
    let g = db::identify::find_group(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    let run = own_run(&state, &family, g.run_id).await?;
    let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (album, artist) = (clean(body.album), clean(body.artist));
    if album.is_none() && artist.is_none() {
        return Err(Problem::bad_request("empty_search", "give an album or an artist").field("/album", "required", "an album or an artist"));
    }
    let source = source(&state)?;
    if identify_runs::search_group(state.db(), &source, &run, id, artist.as_deref(), album.as_deref()).await?.is_none() {
        return Err(Problem::not_found());
    }
    Ok(Json(group_out(&state, &family, id).await?))
}

/// Undo one track's identify: its title, artist, album, album artist, genre, track
/// number and MusicBrainz ids go back to what they were before it was first
/// identified, by a run or on its own. A run's proposal for it is pending again.
/// The file was never touched.
#[utoipa::path(delete, path = "/tracks/{id}/identity", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "The track as it is now", body = crate::v2::music::Track),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`not_identified`, or `nothing_recorded`: identified before the server kept what it replaced", body = crate::v2::problem::ProblemBody)))]
pub async fn revert_track(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<crate::v2::music::Track>, Problem> {
    let viewer = family.viewer();
    let row = db::catalog::find_track(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    if row.user_id != family.user_id {
        return Err(Problem::forbidden().detail("only its owner reverts this track"));
    }
    match db::music::revert_identity(state.db(), id, family.user_id, &crate::db::edits::Edit::new(family.user_id, "revert")).await? {
        db::music::RevertOutcome::Reverted => {}
        db::music::RevertOutcome::NotIdentified => {
            return Err(Problem::new(StatusCode::CONFLICT, "not_identified").detail("this track has not been identified"));
        }
        db::music::RevertOutcome::NothingRecorded => {
            return Err(Problem::new(StatusCode::CONFLICT, "nothing_recorded")
                .detail("this track was identified before the server kept what it replaced; correct it instead"));
        }
    }
    let row = db::catalog::find_track(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    let track = crate::v2::music::tracks(&state, vec![row]).await?.pop().ok_or_else(Problem::not_found)?;
    Ok(Json(track))
}

fn source(state: &AppState) -> Result<Identify<'_>, Problem> {
    source_configured(state)?;
    Identify::from_config(state.config())
        .ok_or_else(|| Problem::internal(anyhow::anyhow!("no MusicBrainz source")))?
        .map_err(Into::into)
}

#[derive(Deserialize, ToSchema)]
pub struct TrackSearch {
    /// What to search for, as the owner typed it: the file's own tags are often
    /// what is wrong ("Track 02", a typo, the artist in the title).
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// At most this many, 1–25; 10 by default.
    pub limit: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct Candidates {
    /// Recordings, best first; the track's own length is part of the score.
    pub items: Vec<MetadataCandidate>,
}

/// Search MusicBrainz for a track with the owner's words. The track's length
/// is added from the library.
async fn search(state: &AppState, duration_secs: Option<i32>, body: TrackSearch) -> Result<Vec<MetadataCandidate>, Problem> {
    let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (title, artist, album) = (clean(body.title), clean(body.artist), clean(body.album));
    if title.is_none() && artist.is_none() && album.is_none() {
        return Err(Problem::bad_request("empty_search", "give a title, an artist or an album")
            .field("/title", "required", "a title, an artist or an album"));
    }
    let duration_ms = duration_secs.filter(|s| *s > 0).map(|s| s as u64 * 1000);
    let limit = body.limit.unwrap_or(10).clamp(1, 25);
    Ok(source(state)?
        .identify(title.as_deref(), artist.as_deref(), album.as_deref(), duration_ms, limit)
        .await?)
}

#[derive(Deserialize, ToSchema)]
pub struct RecordingChoice {
    /// A MusicBrainz recording id, or a musicbrainz.org/recording/… link.
    pub recording: String,
    /// The release to put it on: an id or a musicbrainz.org/release/… link.
    /// Left out, MusicBrainz's own choice for the recording.
    pub release: Option<String>,
}

fn parse_choice(c: &RecordingChoice) -> Result<(String, Option<String>), Problem> {
    let recording = matching::mbid(&c.recording, "recording").ok_or_else(|| {
        Problem::bad_request("invalid_recording", "a MusicBrainz recording id or link")
            .field("/recording", "invalid", "a MusicBrainz recording id or link")
    })?;
    let release = match c.release.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        None => None,
        Some(r) => Some(matching::mbid(r, "release").ok_or_else(|| {
            Problem::bad_request("invalid_release", "a MusicBrainz release id or link")
                .field("/release", "invalid", "a MusicBrainz release id or link")
        })?),
    };
    Ok((recording, release))
}

/// The other recordings stored with a proposal: what its lookup also found, or
/// what the owner's last search found.
#[utoipa::path(get, path = "/identify/proposals/{id}/alternatives", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Candidates), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn alternatives(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Candidates>, Problem> {
    own_proposal(&state, &family, id).await?;
    let items = serde_json::from_value(db::identify::alternatives(state.db(), id).await?).unwrap_or_default();
    Ok(Json(Candidates { items }))
}

/// Search again for a proposal's track with the owner's words. What it finds
/// becomes the proposal's alternatives; nothing is chosen yet.
#[utoipa::path(post, path = "/identify/proposals/{id}/search", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = TrackSearch,
    responses((status = 200, body = Candidates), (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn search_proposal(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<TrackSearch>) -> Result<Json<Candidates>, Problem> {
    let p = own_proposal(&state, &family, id).await?;
    let items = search(&state, p.duration_secs, body).await?;
    db::identify::set_alternatives(state.db(), id, &serde_json::to_value(&items).map_err(anyhow::Error::from)?).await?;
    Ok(Json(Candidates { items }))
}

#[derive(Deserialize, ToSchema)]
pub struct ProposalRecording {
    #[serde(flatten)]
    pub choice: RecordingChoice,
    /// Apply it at once, as an accept would.
    #[serde(default)]
    pub accept: bool,
}

/// Make a recording the owner picked the proposal: one of its alternatives, or
/// any recording by id or link. It is looked up by id, so a pasted id works for
/// a track nothing was found for. Pending again if it was rejected; an accepted
/// proposal is undone first.
#[utoipa::path(put, path = "/identify/proposals/{id}/recording", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = ProposalRecording,
    responses((status = 200, body = Proposal), (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`already_decided`: accepted; undo it first", body = crate::v2::problem::ProblemBody)))]
pub async fn choose_recording(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<ProposalRecording>) -> Result<Json<Proposal>, Problem> {
    let p = own_proposal(&state, &family, id).await?;
    if p.decision == "accepted" {
        return Err(Problem::new(StatusCode::CONFLICT, "already_decided").detail("this proposal was accepted; undo it first"));
    }
    let (recording, release) = parse_choice(&body.choice)?;
    let detail = source(&state)?.recording(&recording, release.as_deref()).await?;
    let found: Vec<MetadataCandidate> = serde_json::from_value(db::identify::alternatives(state.db(), id).await?).unwrap_or_default();
    let hint = found.iter().find(|c| c.mb_recording_id == recording && (release.is_none() || c.mb_release_id == release));
    let proposed = matching::chosen(detail, hint);
    if !db::identify::set_owner_choice(state.db(), id, &serde_json::to_value(&proposed).map_err(anyhow::Error::from)?).await? {
        return Err(Problem::new(StatusCode::CONFLICT, "already_decided").detail("this proposal was accepted; undo it first"));
    }
    if body.accept {
        identify_runs::accept(state.db(), family.user_id, id).await?;
    }
    Ok(Json(proposal_out(own_proposal(&state, &family, id).await?)))
}

async fn own_track(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<db::catalog::TrackRow, Problem> {
    let row = db::catalog::find_track(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    if row.user_id != family.user_id {
        return Err(Problem::forbidden().detail("only its owner identifies this track"));
    }
    Ok(row)
}

/// Search MusicBrainz for any of your tracks, with your own words — for a song
/// a run did not find, or found wrong.
#[utoipa::path(post, path = "/tracks/{id}/identity/search", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = TrackSearch,
    responses((status = 200, body = Candidates), (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn search_track(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<TrackSearch>) -> Result<Json<Candidates>, Problem> {
    let row = own_track(&state, &family, id).await?;
    Ok(Json(Candidates { items: search(&state, row.duration_secs, body).await? }))
}

#[derive(Deserialize, ToSchema)]
pub struct TrackIdentity {
    #[serde(flatten)]
    pub choice: RecordingChoice,
    /// Also fetch the release's cover for the track.
    #[serde(default)]
    pub fetch_cover: bool,
}

/// Identify one of your tracks as a MusicBrainz recording: its title, artist,
/// album, album artist, track number and MusicBrainz ids come from
/// MusicBrainz, looked up by id; its genre stays. `DELETE` undoes it. The file
/// is never touched.
#[utoipa::path(put, path = "/tracks/{id}/identity", tag = "identify", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = TrackIdentity,
    responses((status = 200, description = "The track as it is now", body = crate::v2::music::Track),
        (status = 400, body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn identify_track(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<TrackIdentity>) -> Result<Json<crate::v2::music::Track>, Problem> {
    own_track(&state, &family, id).await?;
    let (recording, release) = parse_choice(&body.choice)?;
    let detail = source(&state)?.recording(&recording, release.as_deref()).await?;
    let track = db::music::find_track_owned(state.db(), id, family.user_id).await?.ok_or_else(Problem::not_found)?;
    db::music::apply_musicbrainz_metadata(
        state.db(),
        id,
        family.user_id,
        &detail.title,
        detail.artist.as_deref().or(track.artist.as_deref()),
        detail.album.as_deref().or(track.album.as_deref()),
        track.genre.as_deref(),
        detail.track_number.or(track.track_number),
        &detail.mb_recording_id,
        detail.mb_release_id.as_deref(),
        detail.mb_artist_id.as_deref(),
        detail.album_artist.as_deref(),
        detail.mb_release_group_id.as_deref(),
        None,
    )
    .await?;
    if body.fetch_cover {
        crate::music::apply_cover_art(&state, &family, id, &detail).await;
    }
    let row = db::catalog::find_track(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let track = crate::v2::music::tracks(&state, vec![row]).await?.pop().ok_or_else(Problem::not_found)?;
    Ok(Json(track))
}
