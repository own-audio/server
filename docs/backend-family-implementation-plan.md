# Backend Implementation Plan: Family Audio Cloud Service

Date: 2026-07-18
Status: **living plan — checklist items get ticked as we go; safe to resume any time.**
Related:
- [backend-gap-analysis-abs-navidrome.md](backend-gap-analysis-abs-navidrome.md)
- [mobile-backend-api-spec.md](mobile-backend-api-spec.md)
- [android-client-guide.md](android-client-guide.md)

## Vision

Turn audio2 into an all-in-one audio cloud service for **families**:

- A family is a user group with as many members as the family wants.
- Inside a family, **family admins** control which audio content (audiobooks,
  podcasts, music) each family member is allowed to listen to.
- Every family member keeps their **own playback memory** (progress, bookmarks,
  queue) and gets a **personalized statistics board**.
- Management/admin happens on the **web desktop** app; consumption happens
  mostly on **mobile — iOS and Android alike** (revised 2026-07-18). ~~The
  backend stays strictly client-agnostic~~ **Superseded 2026-08-02**: `audio2-mac`
  is now the lead client — new endpoints get designed and proven against it
  first, then ported to iOS/Android once the shape is right (see `CLAUDE.md`).
  The client-agnostic *contract* discipline doesn't change (no Apple-only
  assumptions baked into the API itself, both APNs and FCM push,
  background-reporting APIs that still work under Android's Doze/WorkManager
  constraints), but which client's real usage informs a new endpoint's shape
  does: expect Mac-shaped requests first, not blind speculative specs.

## Architecture decisions (locked in unless revisited)

1. **Family = group, not merged accounts.** Users stay individual accounts
   (own credentials, own progress rows — already per-`user_id` everywhere).
   A family wraps them; nothing about existing per-user progress tables
   changes.
2. **Every user has a private space; the family library is opt-in.**
   (Revised 2026-07-18 by decision D1.) Content keeps its `user_id` owner and
   gains a **nullable** `family_id`:
   - `family_id IS NULL` ⇒ **private** to the owner. Invisible to everyone
     else — *including family admins*. This is the safety valve that makes a
     shared family service acceptable for adults.
   - `family_id = X` ⇒ **shared** with family X, subject to the per-member
     policy/grant rules below.

   Sharing and un-sharing are explicit actions on an item. Nothing is ever
   shared retroactively: the Phase 2 backfill sets `family_id = NULL`
   everywhere, so no existing library is exposed by the upgrade.
3. **Access control = default policy + explicit grants,** applied **only to
   shared content**. Each member has a per-media-kind default (`allow_all` or
   `deny_all`) plus item-level overrides. This handles both "kids see only
   what's granted" (deny-all default + grants) and "adults see everything
   except X" (allow-all default + denials). Family admins bypass the grant
   filter — but *within shared content only*; a family admin can never see
   another member's private items.
4. **Roles**: keep global `admin`/`user` (instance operator) and add
   family-scoped roles `family_admin`/`member` in the membership table.
   Global admin ≠ family admin.
5. **Statistics need history, not just latest position.** Add an append-only
   `listening_sessions` table written alongside every progress upsert;
   stats endpoints aggregate from it.
6. **Mobile readiness is part of this plan**, not a later phase: refresh
   tokens, push registration (APNs *and* FCM), play-queue sync, delta sync,
   bulk ops. Nothing in the API may assume a particular platform.

---

## Phase 0 — Auth & session foundations ✅ (2026-07-18)

Prereq for everything mobile. Replaces the "JWT only, re-login weekly" model.

- [x] Migration `0017_refresh_tokens.sql`: `refresh_tokens` table
      (id, user_id, chain_id, token_hash, device_name, device_kind,
      created_at, last_used_at, expires_at, revoked_at, replaced_by) +
      `sessions.chain_id`. Only the SHA-256 of the token is stored.
- [x] `POST /auth/refresh` — rotate-on-use; presenting an already-rotated
      token revokes the whole device chain (reuse = theft).
- [x] `POST /auth/login` returns `{token, refresh_token, user}` and accepts
      optional `device_name`/`device_kind` (`web|ios|android|other`).
      Register + setup/complete return the same shape.
- [x] `GET /auth/sessions` — signed-in devices with `current` marker.
- [x] `DELETE /auth/sessions/{chain_id}` — sign out one device (kills its
      refresh chain AND its access sessions).
- [x] `/auth/logout` revokes the session and its whole refresh chain.
- [x] Config: `auth.access_ttl_secs` (unset ⇒ falls back to
      `session_ttl_secs`, so existing deployments are unchanged until
      clients adopt refresh) + `auth.refresh_ttl_secs` (default 90 days).
- [x] Smoke-test coverage: login → refresh → rotation → reuse-detection →
      chain revocation → middleware rejection of revoked access tokens.
- [x] Bonus fixes surfaced by this work:
      - `AuthUser` middleware now actually checks `sessions.revoked_at`
        (revocation was never enforced before — admin revoke and logout
        were cosmetic).
      - `change_password` now truly keeps the current session
        (`revoke_all_except`) — previously it revoked ALL sessions,
        contradicting its own comment.
      - Smoke test: media probes use ranged GET instead of HEAD (presigned
        S3 URLs are GET-signed; HEAD always 403s), and the Subsonic
        stream-redirect check uses a key owned by the track's owner.

## Phase 1 — Families & membership ✅ (2026-07-18)

- [x] Migration `0018_families.sql`: `families`, `family_members`
      (UNIQUE user_id ⇒ one family per user in v1, roles
      `family_admin|member`, optional `display_label`), `family_invites`
      (email-bound bearer `code`, 14-day TTL, `accepted_at/accepted_by`).
- [x] Backfill in the same migration: every existing user becomes the
      `family_admin` of a personal family of one, so handler code can assume
      a family always exists.
- [x] Router `crate::families` at `/api/v1/family`:
      `GET /`, `PUT /` (rename), `GET /members`,
      `PUT /members/{user_id}` (role and/or display_label),
      `DELETE /members/{user_id}` (remove, or leave when targeting self),
      `GET|POST /invites`, `DELETE /invites/{id}`, `POST /invites/accept`.
- [x] `FamilyContext` extractor resolves `family_id` + `family_role` per
      request, lazily creating a personal family for any account that
      somehow lacks one. Instance admins count as family admins.
- [x] Invite-aware registration: `POST /auth/register` accepts
      `invite_code`, which authorizes the account **even when open
      registration is disabled** and drops it straight into the inviting
      family. Verified: code is email-bound (wrong email → 400), single-use
      (reuse → 400), and bogus/expired codes are rejected.
- [x] Guards: last family admin cannot be demoted or removed while other
      members remain; removing a member re-homes them into a fresh personal
      family (never orphaned); emptied families are pruned.
- [x] Smoke-test coverage: personal-family bootstrap, rename, invite →
      register-with-code → membership, reuse rejection, member-vs-admin
      permission checks (403s), label editing, removal + re-homing.
- **D1 was revisited during Phase 2 and decided the other way: a joiner's
  library stays private.** Every user keeps a private folder and shares
  individual items deliberately; see the privacy model in "Architecture
  decisions" above.

## Phase 2 — Family library & per-member access control ✅ (2026-07-18)

The core feature. Touches every content module.

- [x] Migration `0019_family_content.sql` (2026-07-18):
  - Nullable `family_id` (FK → families, ON DELETE SET NULL) on
    `audiobook_books`, `podcast_feeds`, `music_tracks`, `music_playlists`,
    `audiobook_collections`, `audiobook_series`, with partial indexes on
    the shared rows. `user_id` stays as the owner.
  - No backfill — every existing item stays private. Verified on the dev
    database after migrating: 1 private / 0 shared.
  - `member_media_policy` (missing row ⇒ `allow_all`) and `content_grants`
    (UNIQUE per user+kind+item) as specified.
  - **`audio2_can_access(...)` SQL function** holds the visibility rule so
    REST, Subsonic, and list queries cannot drift apart.
- [x] `db::access` (2026-07-18): `VISIBLE` predicate for list queries,
      `can_listen` point check, `set_shared` / `unshare_all_for_user`,
      policy + grant CRUD (bulk `replace_grants`), and `item_audience`
      (reverse "who can hear this" view).
- [x] Rule verified by 10 SQL assertions run in a rolled-back transaction —
      notably **a family admin cannot see another member's private item**,
      grants override policy in both directions, cross-family access is
      denied, and an owner keeps access to their own content even if a
      `deny` grant names them.
- [x] Leaving/being removed from a family reverts that member's shared
      items to private and clears their grants and policies.
- [x] **Music module reworked (2026-07-18)** — the template for the rest:
  - Read paths (`list_tracks`, `find_track`, playlists) apply the
    visibility predicate; write paths use new `*_owned` variants so only
    the owner can edit, delete, or re-share an item.
  - `stream_track` authorizes through the same lookup, so a presigned URL
    is never issued for content the caller cannot play.
  - Upload accepts a `visibility` field (`private` | `family`, absent ⇒
    private); `PUT /music/tracks/{id}/visibility` and
    `PUT /music/playlists/{id}/visibility` move items between folders.
  - Responses carry `visibility` and `is_owner` so the UI can show the
    right folder and hide edit controls on someone else's shared item.
- [x] **Subsonic surface scoped the same way** — `getArtists`,
      `getAlbumList2`, `getAlbum`, `search3`, `stream`, `getCoverArt`, and
      playlists all evaluate `audio2_can_access`. Playlists created over
      Subsonic default to private. The compiler forced every call site to
      be updated, which is exactly why the rule lives in one function.
- [x] Verified end-to-end by `scripts/family_sharing_test.py`: a family of
      three, private upload invisible to the other two (including over
      direct fetch and stream), sharing makes it visible and streamable,
      non-owners cannot edit/delete/re-share it, un-sharing revokes access,
      and the owner leaving reverts their content to private.
- [x] **Audiobooks and podcasts reworked (2026-07-18)** on the same pattern:
      read paths take a viewer, mutations take `find_*_owned`, uploads and
      `POST /audiobooks` / `POST /podcasts/subscribe` accept `visibility`,
      and `PUT /{audiobooks,podcasts}/{id}/visibility` moves items between
      folders. Responses carry `visibility` + `is_owner`.
- [x] `library::continue` and `library::search` evaluate the same rule.
      **Search now also returns music tracks**, closing a gap-analysis item
      (it previously covered only feeds, episodes, and books).
- [x] Parental-control API under `/api/v1/family` (family_admin only):
  - `GET /members/{id}/access` — effective policies + grants
  - `PUT /members/{id}/policy` — per-kind `allow_all` / `deny_all`
  - `PUT /members/{id}/grants` — bulk replace item-level allow/deny
  - `GET /content/{kind}/{id}/audience` — who can currently hear an item
- [x] `GET /library/private` — the caller's never-shared items across all
      kinds, for a "Soukromé" section. Always self-scoped: there is no API
      path to read another member's private folder, family admin included.
- [x] Extended `scripts/family_sharing_test.py` covers all of it: audiobook
      private→shared, `deny_all` hiding shared books from the kid while
      leaving the adult untouched, an allow-grant restoring one title, a
      deny-grant removing it again, the audience view reflecting both, a
      restricted member being unable to read or lift their own limits, and
      private items staying out of other members' search results.
- [ ] Admin endpoints (family_admin only), nested under `/api/v1/family`:
  - `GET /members/{user_id}/policy` / `PUT .../policy` — set per-kind
    default policy
  - `GET /members/{user_id}/grants?kind=` — list effective grants
  - `PUT /members/{user_id}/grants` — bulk replace grants for a kind
    (the web UI will present checkboxes; bulk-replace beats item-by-item)
  - `GET /content/{kind}/{item_id}/audience` — reverse view: which members
    can hear this item (drives the "who can listen" widget on detail pages)
- [ ] Uploads: new content defaults to visible per each member's default
      policy; response includes an `audience` summary so the web UI can
      immediately offer "restrict this".
- [ ] Playlists: personal by default (only creator + family admins see
      them), with an `is_shared` flag to publish to the family. Collections
      already have `is_public` — unify semantics ("public" = family-visible).
- [ ] Per-member progress/bookmarks/settings: **no change needed** (already
      keyed by user_id) — verify with tests that two members of one family
      have fully independent progress on the same shared book.
- [ ] Subsonic `/rest`: scope to the family library filtered by the caller's
      grants (the API key already maps to a user).

## Phase 3 — Listening history & statistics board ✅ (2026-07-18)

- [x] Migration `0020_listening_stats.sql`: `listening_sessions` (append-only
      span log with `device_kind` covering web/iOS/Android/Subsonic, a
      `source` of `reported`|`derived`, and a `client_session_id`
      idempotency key), `listening_daily` rollup, and
      `family_members.stats_visibility`.
- [x] `POST /playback/sessions` — **batched** (up to 500 spans) and
      **idempotent**. Both matter on mobile: Android WorkManager and iOS
      background tasks retry aggressively, and a device that was offline or
      in Doze flushes everything at once. A replayed batch records nothing.
- [x] Sessions are also **derived from progress saves**, so today's web UI
      and Subsonic clients produce statistics with no client change.
      Derivation counts only forward deltas, ignores rewinds, and switches
      off for users whose clients report explicitly — that is what keeps the
      two sources from double-counting.
- [x] Stats endpoints: `GET /stats/me` (totals, per-kind split, per-day
      series, streak, top items with resolved titles),
      `GET /stats/me/history`, `GET /stats/family` (family_admin).
      Day bucketing honours a `tz_offset_minutes` parameter, so "yesterday"
      means the listener's yesterday rather than the server's.
- [x] `stats_rollup` job rebuilds `listening_daily` idempotently, so a late
      offline batch corrects earlier days instead of being lost. Verified
      end-to-end against the running worker.
- [x] **D2 decided: listening history is private by default.** A family
      admin sees a member's figures only if (a) the member opted in via
      `PUT /stats/me/visibility`, or (b) the member is *restricted* — they
      carry a `deny_all` policy, i.e. a managed/child account — in which
      case `PUT /stats/family/members/{id}/visibility` may enable it without
      consent. A parent therefore cannot silently start watching another
      adult. Hidden members still appear in the roll-up as `hidden: true`
      with no figures, so the roster renders without leaking totals.
- [x] Covered by `scripts/stats_test.py`: batching, retry idempotency,
      per-device attribution, timezone independence of totals, input
      validation, all four derivation behaviours, and every privacy rule.
## Phase 4 — Mobile-client enablement (iOS + Android) ✅ (2026-07-19)

Everything §11 of the mobile spec flagged, minus refresh tokens (Phase 0) and
session reporting (Phase 3). All of it is platform-neutral; where iOS and
Android differ (push transport, background scheduling) the backend carries
both.

- [x] Migration `0021_mobile_sync.sql`: `play_queues`, `deleted_items`
      (tombstones), `device_push_tokens` (apns + fcm), `pending_notifications`.
- [x] **Play queue sync**: `GET|PUT /playback/queue`, last-write-wins with an
      `updated_at` echo and `updated_by_device` so a client can show "playing
      on your phone". An out-of-range `current_index` is clamped rather than
      rejected — a queue trimmed elsewhere must not strand the client.
      Subsonic `savePlayQueue`/`getPlayQueue` share the same row, so a
      Subsonic app and the native clients agree on one queue.
- [x] **Delta sync**: `GET /library/changes?since=` returns changed
      audiobooks, podcasts, and tracks plus **deletion tombstones** in one
      request. Omitting `since` yields a full snapshot, so a first run needs
      no special case. The cursor is taken *before* the queries run, so a row
      written mid-request is re-sent rather than lost.
- [x] Tombstones are recorded on every delete path and pruned after 90 days
      by the `stats_rollup` job; a client offline longer than that does a
      full resync.
- [x] **Push registration**: `POST|DELETE /devices/push-token`,
      `GET /devices/push-tokens`. Tokens are globally unique, so
      re-registering moves a device between accounts instead of duplicating
      it, and listing never echoes a full token back.
- [x] **Notification inbox**: `GET /devices/notifications` +
      `/notifications/ack`. New episodes found by `feed_refresh` queue a
      notification for the feed's owner and for family members allowed to
      play it. This is also the **polling fallback**: until a deployment
      configures APNs/FCM credentials, clients poll on resume and lose
      nothing.
- [x] **Grouped music browsing**: `GET /music/artists`, `/music/albums`
      (optionally `?artist=`), `/music/genres` — so a phone client does not
      reimplement grouping over a flat track list.
- [x] **Bulk operations**: `POST /playback/episodes/progress/bulk` marks many
      episodes played/unplayed in one round trip.
- [x] Covered by `scripts/mobile_sync_test.py` (32 checks).
- [x] Bug found and fixed by that test: `/library/changes` handed out
      `+00:00`-form timestamps, and a client echoing one back as `since`
      would have it mangled into a space by query-string decoding, producing
      a 400 on every second sync. Cursors are now emitted in `Z` form, and
      the parser also tolerates the mangled variant.

### Deliberately not done in this phase

- **Actual push delivery.** Registration, the notification queue, and the
  triggers are in place, but no APNs/FCM sender is implemented — writing an
  untestable transport against credentials we do not have would be worse than
  the honest gap. The inbox makes clients fully functional meanwhile; adding
  a sender later needs no client change.
- **Chapter extraction from embedded tags** (still the top priority in the
  mobile spec's §12.8) and **podcast auto-download rules**. Both are real
  work rather than plumbing, and belong with their own verification.
- **ETag/If-None-Match** on list endpoints — `/library/changes` already
  solves the bandwidth problem it was meant to address.

## Phase 5 — Hardening & remaining gap-analysis items

Ordered by value; the tail items are optional/backlog.

- [ ] **OIDC for real** (Google + Microsoft): implement the stub redirect/
      callback handlers (config structs already exist), identity linking
      table already exists (`find_identity_for_local` implies identities
      table). Mobile note: iOS will use ASWebAuthenticationSession against
      the same redirect flow.
- [ ] **Session/statistic privacy review** + rate limiting on auth
      endpoints (login, refresh, invite accept).
- [ ] **Real scrobbling relay** (Last.fm/ListenBrainz) fed from
      `listening_sessions` (music kind only), per-user opt-in tokens.
- [ ] **Transcoding/bitrate ladder** (biggest lift — likely `ffmpeg`
      sidecar job producing an AAC low-bitrate rendition stored next to the
      original; `/stream?quality=low`): keep as backlog until real users
      hit cellular pain, since presigned-original streaming works.
- [ ] **Star/rating for music** (`music_favorites` table + REST + Subsonic
      `star`/`unstar`/`getStarred2`/`setRating`).
- [ ] **Genre/discovery Subsonic endpoints** (`getGenres`,
      `getRandomSongs`, `getSongsByGenre`, `getNowPlaying` from
      listening_sessions).
- [ ] **DB backup job** (`pg_dump` to S3 on schedule, admin-triggerable).
- [ ] **Webhooks** (generic per-family webhook URLs on events) — backlog.
- [ ] Explicitly **out of scope** (documented, not planned): filesystem
      library scanning (we are upload/subscribe by design), e-book support,
      internet radio, multi-folder libraries.

---

## Phase 6 — Creators Zone support

**New (2026-08-02).** `audio2-mac` is planning a "Creators Zone" — a publishing-focused area
separated into three kind-specific studios (audiobook, podcast, music) rather than one generic
upload screen, since each kind's publishing workflow is genuinely different. Mostly this reuses
what already exists (per-kind upload, metadata editing, `/family/content/{kind}/{id}/audience`)
behind a client-side reorganisation — **most of Phase 6 is "confirm the existing contract
supports the studio, don't build new endpoints speculatively."**

- [ ] Audit existing per-kind upload/metadata/visibility endpoints against what each studio
      actually needs; only add what a real client-side attempt proves is missing — this phase
      should stay small precisely because the endpoints mostly already exist
- [ ] **Open question, not decided:** does v1 need audience/listen analytics per owned item
      (who's actually listening, not just who's *allowed* to), or does content management alone
      cover the first cut? `audience` today answers permission, not usage. If wanted, this is new
      work: an aggregation over `listening_sessions` scoped by content owner rather than by the
      listener themselves — a different access pattern than anything `stats/` does today, worth
      its own design pass rather than bolting onto the existing per-user stats endpoints.
- [ ] If audience analytics are wanted: a `GET /{kind}/{id}/audience-stats`-shaped endpoint,
      owner-only, aggregating `listening_sessions` by content id — sizing depends on the answer
      to the open question above

**Verify:** `audio2-mac`'s three studios each function against existing endpoints with no new
server code, or (if analytics are wanted) a real owner sees real listen counts for their own
content and nothing for content they don't own.

## Phase 7 — Communities (chat & boards)

**New (2026-08-02).** A simplified, Facebook-shaped social layer: chat plus threaded discussion
boards. `audio2-mac`'s own plan (`IMPLEMENTATION_PLAN.md` Phase 13) is blocked on this phase —
coordinate before either side gets ahead of the other. This is genuinely new backend surface,
not a reorganisation of something that already exists like Phase 6 above.

Default scope, matching every existing access-control decision in this product (see
"Architecture decisions" above): **family-scoped.** A chat thread or board lives inside one
family, visible to its members under the same access rules as shared library content — no new
privacy model to design, reuse the one that already exists.

- [ ] **Schema**: `chat_threads` (family-scoped, 1:1 or group), `chat_messages` (thread_id,
      sender, body, created_at — soft-delete for moderation, never hard-delete another member's
      message history), `boards` (family-scoped, optionally `content_kind`/`content_id` to
      attach a board to a specific audiobook/podcast/track), `board_posts` (board_id, author,
      body, parent_post_id for threading)
- [ ] **Access control**: reuse family membership, not a new grant system — every member of a
      family can read/post; `family_admin` gets pin/lock/delete (mirrors the bypass semantics
      `family_admin` already has elsewhere, not a new permission model)
- [ ] **Endpoints**: `GET/POST /families/{id}/chat/threads`, `GET/POST
      /chat/threads/{id}/messages`, `GET/POST /families/{id}/boards`, `GET/POST
      /boards/{id}/posts` — REST/poll-based for v1, not real-time (see the open question below)
- [ ] **Notifications integration**: new messages/posts surface through the existing
      notifications inbox (already client-integrated on iOS/Mac) rather than inventing a second,
      separate unread-count system
- [ ] Moderation audit trail: who deleted/pinned/locked what, for `family_admin` accountability

**Open decisions, not yet settled:**
- **Real-time delivery.** Push-based (WebSocket/SSE) chat is a materially bigger lift than
  poll-based, and this backend has no real-time infrastructure today. Proposal: ship poll-based
  first (matches how notifications already work — "Explicit reporting only" pattern elsewhere in
  this doc), revisit real-time only if poll latency proves to be a real complaint, not
  speculatively.
- **Cross-family communities.** Family-scoped is the default per the architecture decisions
  above, but the user's own framing ("communities," plural) may imply something broader —
  opt-in public boards spanning families — later. Don't build the family-scoped schema in a way
  that forecloses this (e.g. don't hardcode a single `family_id` foreign key path if a nullable
  one that later admits a "public" board is nearly as cheap), but don't build the broader version
  speculatively either.
- **Attachments.** Can a chat message or board post carry an image, or reference a piece of
  library content beyond the board-level `content_kind`/`content_id` attachment above (e.g. "look
  what I'm listening to" sharing a specific timestamp)? Out of scope for a first cut; note it so
  the schema doesn't have to be reworked if it's wanted soon after.

**Verify:** two members of the same family exchange chat messages and post to a shared board; a
third account outside the family can reach neither via direct id guessing (404, matching this
repo's own "missing or not visible" rule in §6) nor via any list endpoint.

---

## Open decisions to settle before/while implementing

| # | Decision | Proposal |
|---|---|---|
| ~~D1~~ | ~~Invite-accept: what happens to the joiner's existing personal library?~~ | **Decided 2026-07-18: it stays private.** Every user keeps a private space; content is shared into the family only by explicit action. See "Privacy model" below. |
| D2 | Family-admin visibility into adult members' stats | Auto-visible only for members with any `deny_all` policy (managed accounts); others opt in. |
| D3 | Per-episode grants for podcasts? | No — whole-feed granularity for v1. |
| D4 | Multi-family membership per user? | No — UNIQUE(user_id) in v1. |
| D5 | Access TTL after refresh tokens land | 1h access / 90d refresh, rotate-on-use. |
| D6 | Where family billing/quotas live (S3 bytes per family?) | Track `size_bytes` per family (media_objects already store size); enforcement later. |
| ~~D7~~ | ~~Default visibility for **new** uploads once families exist~~ | **Decided 2026-07-18: the upload picks its destination.** The user uploads *into* a folder — "Soukromé" or "Rodinná knihovna" — so `visibility` is an explicit field on every upload, not a hidden default. When the field is absent the server falls back to `private` (fail closed: a forgotten field must never expose content, and it keeps today's web client behaving exactly as before). |
| D8 | Communities: real-time (WebSocket/SSE) or poll-based chat delivery for v1? | Proposal: poll-based first, matching how notifications already work here — no real-time infra exists today, revisit only if latency proves a real complaint. |
| D9 | Communities: family-scoped only, or a broader opt-in cross-family layer eventually? | Not decided. Default to family-scoped (matches every existing access decision above), but don't foreclose the schema on a later "public" tier — see Phase 7's own open questions. |

## Suggested implementation order & sizing

| Phase | Est. relative size | Depends on |
|---|---|---|
| 0 Auth foundations | S–M | — |
| 1 Families | M | 0 (nice-to-have, not hard dep) |
| 2 Access control | **L** (touches every module) | 1 |
| 3 Stats | M | 1 (uses family for roll-ups) |
| 4 iOS enablement | M–L (parallelizable per bullet) | 0; queue/push independent of 2 |
| 5 Hardening | varies | mostly independent |
| 6 Creators Zone support | S (mostly an audit, not new work) | 2 (reuses access control) |
| 7 Communities | **L** (new schema, new endpoints, new moderation model) | 1, 2 |

Phases 0, 3, and most of 4 are independent of the family work and can be
interleaved. The critical path for the family product is 1 → 2. **Phase 6 is
deliberately small** — most of what a Creators Zone needs already exists;
resist scope creep into audience analytics unless a real client proves it's
needed. **Phase 7 is the one worth treating with the same care Phase 2 got**
— it's new schema and a new privacy surface, and getting the family-scoping
wrong here has the same "every later screen inherits the bug" property Phase
2 had.

## Working agreement

- Each checklist item lands with: migration (if any) + handler(s) + a smoke
  test entry in `scripts/backend_smoke_test.py`.
- Update this file's checkboxes in the same commit as the feature.
- New endpoints get added to
  [mobile-backend-api-spec.md](mobile-backend-api-spec.md) as they land so
  the iOS client spec stays truthful.
