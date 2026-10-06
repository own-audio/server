# Music Signals & Smart Playlists — backend plan

The backend half of a feature that spans six repositories: measure how people
actually listen, measure what tracks actually sound like, and turn both into
playlists that build themselves.

Written 2026-09-21. `CLAUDE.md` governs everything here.

**Companion documents** — each repo holds only its own part:

| Repo | Document | Owns |
|---|---|---|
| `audio2` (this one) | this file | schema, jobs, endpoints, scoring, playlist evaluation, **all audio analysis** |
| `audio2-mac` | `SMART_PLAYLISTS_PLAN.md` | skip instrumentation, feedback UI, the opt-in setting, on-device intent |
| `audio2-win` | `SMART_PLAYLISTS_PLAN.md` | skip instrumentation, feedback UI, server intent |
| `audio2-ios-music` | `SMART_PLAYLISTS_PLAN.md` | skip instrumentation, feedback UI, on-device intent |
| `audio2-android-music` | `SMART_PLAYLISTS_PLAN.md` | skip instrumentation, feedback UI, server intent |
| `audio2-tvos` | `SMART_PLAYLISTS_PLAN.md` | consumption, feedback, skip reporting |

**No client measures audio (decided 2026-09-21).** An earlier draft of this plan
put analysis in the Mac and Windows apps, because the files are already local
there and the transfer is free. That is reversed: every measurement happens
server-side, on one extractor, in one place.

What that buys is worth more than the saved transfer:

- **The parity problem disappears.** Two desktops running slightly different
  extractor versions would have produced values that are not comparable, and the
  percentile scale in §3.3 would have degraded silently, with nothing failing and
  no error to find. That entire class of bug is gone.
- **The licensing problem disappears** for GPL tooling — see §3.7.
- **The riskiest client milestone disappears.** Getting a native extractor
  through MSIX on x64 and ARM64 was the one item in `audio2-win` that could have
  failed outright.

What it costs is transfer time, which §3.4 makes bounded and deliberate.

**Prerequisite relationship to `audio2-mac/MUSIC_AI_STUDY.md`:** that study
(2026-08-28) proposes a `searchTracks(mood:, genre:, energy:)` tool for the
on-device model, and a Spike A to judge output quality against a real library
slice. **That tool cannot be built today** — no track in this system has an
energy, a tempo, or a mood. Parts A and B below are what make Spike A runnable
against real data instead of hand-picked samples. The study decides *whether*
the AI layer is worth shipping; this plan builds the substrate it would sit on,
and is worth building even if the study's verdict is no.

**Legend:** ✅ done · 🚧 in progress · ⬜ not started

**Progress: 9 / 9 milestones — S0–S8 ✅. Version 0.1.15, deployed.**

---

## 1. The premise

A family library is not a streaming catalogue, and the difference changes what
is worth building.

1. **It is small** — thousands of tracks, not a hundred million.
2. **It is entirely deliberate** — every track in it was acquired by someone on
   purpose. There is no filler.

So "discovery" here does not mean *recommend something unknown*. It means
**resurface something forgotten**. Of twenty thousand tracks, a few hundred are
in active rotation; the rest is the product. That problem does not need
collaborative filtering, embeddings, or anyone else's taste graph — it needs
listening history, which this backend already collects.

A second correction worth stating plainly, because it shapes the whole design:
**a shared library is not a shared taste.** The schema already splits these
correctly — `music_tracks.family_id` makes the pool shared (migration 0019),
while `music_track_stars`, `music_track_ratings` and `listening_sessions` are
all keyed per `user_id`. Every personalised thing below stays per-user over a
shared pool. Nothing in this plan computes a "family taste."

---

## 2. Part A — the signal layer

### 2.1 What exists today

| Signal | Where | Per-user |
|---|---|---|
| Favourites (track, artist, album) | `music_track_stars`, `music_group_stars` (0036) | yes |
| Star rating 1–5 | `music_track_ratings` (0036) | yes |
| Listening history | `listening_sessions` + `listening_daily` (0020) | yes |
| Last position / completed | `music_progress` (0012) | yes |

Stats privacy (decision D2 in 0020) already says a member's history is personal
and family admins do not see it by default. **Everything in Part A inherits that
rule.** A per-user preference weight is at least as personal as a play count.

### 2.2 What is missing: the skip

`listening_sessions` records `seconds_listened`, but nothing records *why the
session ended*. A twelve-second session might be a skip, a phone call, a crashed
app, or someone sampling a track on purpose. Only the client knows.

This is the single missing primitive. Everything else in this document is built
on it, so it lands first.

### 2.3 `ended_reason`

Add to `listening_sessions`:

```sql
ALTER TABLE listening_sessions
    ADD COLUMN ended_reason TEXT
        CHECK (ended_reason IN ('completed', 'skipped', 'stopped', 'replaced'));
```

Nullable — every existing row, and every client that has not been updated, keeps
working and simply contributes no preference signal.

| Value | Meaning |
|---|---|
| `completed` | played to the end (or within the last few seconds) |
| `skipped` | user explicitly advanced to another track |
| `stopped` | playback ended without a skip — paused, quit, interrupted |
| `replaced` | user navigated away to something unrelated, not a next-track skip |

**A skip is not a skip.** Position matters more than the fact:

- skipped in the first ~15 % of the track → a genuine "not this"
- skipped after ~70 % → the track effectively played; near-zero signal
- between → partial

The clients send the position; the server derives the weight from
`seconds_listened / duration_secs`. The clients do not send a judgement, only
facts — so the curve can be re-tuned later without shipping five app updates.

`POST /playback/sessions` (`report_sessions`, `src/playback/mod.rs:306` →
`db::stats::record_sessions`, `src/db/stats.rs:73`) gains the optional field.
Sessions derived from progress updates (`derive_from_progress`) leave it NULL —
they are inference, not observation, and must not be mistaken for one.

### 2.4 Explicit signals: two buttons, not one

The user asked for both, and conflating them would be a mistake.

| Button | Meaning | Storage | Recovers |
|---|---|---|---|
| **"Not feeling it"** (dislike) | a strong negative preference | weight × 0.1 | slowly |
| **"Never play this again"** | remove from all automatic selection | hard exclusion | never, until undone |

```sql
CREATE TABLE music_track_feedback (
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id   UUID        NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    kind       TEXT        NOT NULL CHECK (kind IN ('dislike', 'banned')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);
```

**`banned` never deletes anything.** The track stays in the family library and
stays visible to everyone else — it is excluded from *this user's* automatic
selection only. This is the same distinction `CLAUDE.md` draws elsewhere between
private and shared: "not for me" is a preference, not a permission, and it gets
no lock icon and no deletion.

Both are reversible from the track's context menu, and both must be visible
somewhere as a list — a user who banned something by accident needs to find it
again.

### 2.5 The weight model

One number per (user, track), in `[0, 1]`, multiplied into every automatic
selection.

```
weight = clamp(base
               × skip_penalty(recovered)
               × feedback_penalty
               × recency_penalty,
               0, 1)
```

- **base** — 1.0, raised by explicit affection: starred → 1.3, rating 4–5 → 1.2,
  rating 1–2 → 0.6 (clamped afterwards).
- **skip_penalty** — each early skip multiplies by 0.5; each completion
  multiplies back toward 1.0. **It recovers with time**: a skip's effect decays
  with a half-life of ~4 weeks. The user's own framing, and it is the right one:
  *a track that did not suit the moment is not a track you never want to hear
  again.* That is what the ban button is for.
- **feedback_penalty** — 0.1 for `dislike` (also recovering, but over months);
  `banned` is not a penalty, it is a filter applied before scoring.
- **recency_penalty** — played in the last N days → temporarily down-weighted,
  so rotation does not collapse onto the same forty tracks.

None of these constants are defensible from first principles. They are starting
values to be tuned against one real library, and they live in one place in the
code with that fact written next to them.

### 2.6 The rollup

`listening_sessions` is an append-only log. Aggregating it on every queue refill
is wrong. One rollup table, maintained by the existing `stats_rollup` job:

```sql
CREATE TABLE music_track_affinity (
    user_id        UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id       UUID        NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    play_count     INTEGER     NOT NULL DEFAULT 0,
    skip_count     INTEGER     NOT NULL DEFAULT 0,
    early_skips    INTEGER     NOT NULL DEFAULT 0,
    last_played_at TIMESTAMPTZ,
    last_skipped_at TIMESTAMPTZ,
    weight         REAL        NOT NULL DEFAULT 1.0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);
```

`weight` is stored because it is read constantly and recomputed rarely. The raw
counters are stored alongside it so the formula can change without replaying the
session log.

---

## 3. Part B — the feature layer

### 3.1 MusicBrainz does not have this data

Worth recording, because it is a natural assumption and it is wrong.
MusicBrainz is a database of **identity and relationships**: who recorded what,
on which release, how long it is, ISRC, credits. It has never held tempo, key,
loudness, energy or mood — those are signal properties, not facts about a
release.

The mirror on the NUC additionally carries only the **CC0 core + cover-art
index** (`music-metadata/docs/deployment.md`), so even MusicBrainz's own
folksonomy tags and curated genres — which live in `mbdump-derived` — are not
loaded. Verify before assuming otherwise:

```sql
SELECT (SELECT count(*) FROM musicbrainz.genre)         AS genres,
       (SELECT count(*) FROM musicbrainz.recording_tag) AS rec_tags;
```

AcousticBrainz is the one ready-made source, keyed by the same recording MBID
`music_tracks.musicbrainz_recording_id` already stores (0030) — but it stopped
collecting in 2022, its coverage is whatever users once submitted, and its
high-level mood/genre labels were trained on small datasets and are not
trustworthy. Its low-level values (tempo, key, loudness) are sound, because
Essentia computed them. **Deploying a second dump to get a fraction of the
library, then still needing local analysis for the rest and for every new
upload, is two pipelines where one will do.** Not recommended.

Spotify's audio-features API, the historical shortcut, closed to new
applications in November 2024.

So: we compute it ourselves, from files we already hold. That is a capability no
streaming service's customer has.

### 3.2 What to measure

| Column | Kind | Reliability |
|---|---|---|
| `bpm` | measured | high on steady-beat music; fails on rubato classical, free jazz, ambient. Watch for octave errors (70 vs 140) |
| `music_key`, `key_scale` | estimated | moderate; useful for sequencing, not for filtering |
| `loudness_lufs` | measured | exact (EBU R128) |
| `dynamic_range` | measured | exact |
| `spectral_centroid` | measured | exact |
| `onset_rate` | measured | exact |
| `analysis_raw` | `JSONB` | the extractor's full output, verbatim |
| `analysis_version` | `SMALLINT` | which extractor/formula produced the row |

### 3.3 Store raw measurements, derive energy

**Energy is not a physical quantity.** Spotify's was a proprietary blend and
there is no ground truth to match. This matters because it makes the problem
*easier*: we do not need a correct value, we need a scale that is **consistent
and monotonic within one library**. A weighted blend of loudness, spectral
centroid, onset rate and dynamic range, normalised to a percentile across the
library, is entirely sufficient for "give me 0.4–0.7".

Two consequences, both load-bearing:

1. **Never store only the derived energy.** It is a percentile, so it moves every
   time the library grows. Store the raw measurements permanently and compute
   energy over them — in a materialised view refreshed nightly, or at query
   time. Otherwise every new upload would require re-decoding everything.
2. **One extractor, one formula, one place.** If several platforms each measured
   and each derived "energy," the column would hold several definitions of the
   same number and the percentile scale would be quietly meaningless — nothing
   failing, results merely getting worse. That risk is what §3.4's server-only
   decision removes at the root.

### 3.4 Where the numbers come from

Three sources, one storage shape, in order of preference:

One source: the `analyze_audio` job, fetching from R2. Two triggers.

### Trigger 1 — the opt-in backfill

**A family turns the feature on, and its whole library is measured once.**

This is the cold-start answer. Analysis on demand is circular — a query cannot
filter by energy without knowing the energy of the tracks it is filtering. And
analysing lazily, as tracks happen to be played, would make the percentile in
§3.3 a percentile over whatever was played first, which is a biased sample that
shifts under everyone as coverage grows.

A full pass fixes both: the population is the library from day one.

Four things this needs, and the third is the non-obvious one:

1. **It is a family setting, not a personal one.** The checkbox is per family
   because the data is per track. Measurements are a property of the recording,
   not of a listener, so the first member to enable it measures the shared
   library for everyone. Private tracks (`family_id IS NULL`) are measured only
   for their owner.
2. **It is a question about work, not about privacy.** Nothing leaves the
   server, nothing is shared, no third party is involved. The wording users see
   should say "this will process your whole library once," not anything that
   sounds like a data-processing consent — it would read as more alarming than
   it is.
3. **Order by play count, descending.** Not by id, not by date added. The
   feature becomes useful at five per cent coverage instead of a hundred,
   because the tracks people actually play are the ones any early playlist will
   draw on. A cheap `ORDER BY` with a large payoff.
4. **Turning it off stops new work but keeps existing rows.** Deleting
   measurements would mean re-fetching the library if the user changed their
   mind. The cost is that "off" does not mean "erased"; say so plainly rather
   than pretending otherwise.

### Trigger 2 — the periodic sweep

New uploads, Subsonic imports, anything that arrived since the last pass. The
sweep finds tracks with no current measurement and enqueues them in capped
batches — exactly what `media_checksum` already does.

**The sweep needs a guard the checksum sweep does not have.** `media_checksum`
runs unconditionally for every object. This one must consider only tracks
belonging to families that enabled the feature, or the opt-in is decoration and
the library gets measured either way. This is the easiest thing to get wrong by
copying that job too faithfully.

### What this costs, stated plainly

Opting in does not avoid the ~200 GB fetch; it makes it deliberate and bounded
to people who asked for it. In practice that is one family and one library,
once. That is fine — but it should not be a surprise the first time it runs, so
it needs a progress indicator and a cancel.

Surviving a restart is free: the queue is a query
(`WHERE analysis_version IS NULL`), so state lives in the data.

**Range requests now matter more, not less.** Fetching ~1.5 MB from the middle
of a file instead of the whole thing was a minor optimisation when a desktop did
the work. It is now the main lever on what the server pulls. MP3 and AAC decode
from a mid-file range because their frames self-synchronise; FLAC and M4A need
the header too, so either two ranges or a full fetch for those.

Production storage is **R2** (`deploy/production/env.example:27`); Garage is
local development only (`docker-compose.yml`). R2 egress is free and twenty
thousand Class B operations cost a fraction of a dollar, so the backfill is not
a money problem — it is a **transfer-time** problem, roughly 200 GB pulled to
wherever the compute runs. Both client paths avoid it entirely.

**The sha256 path is the interesting one.** `media_objects.sha256` (0039) is
already computed for every object and indexed. A local tool on the Mac can hash
an original file, find the exact track, and POST the measurements — a few
hundred bytes per track instead of ten megabytes. Matching is exact; no guessing
by title and duration. Whatever does not match falls back to the server job,
and that remainder will be a handful, not twenty thousand.

**Because a second full pass is expensive, extract everything on the first
pass.** Not just tempo — every column in §3.2 plus the extractor's raw JSON.
Postgres storage is cheap; re-fetching the library is not. `analysis_version`
then exists to re-derive a *formula*, which is pure SQL, rather than to fetch
*data* again.

### 3.5 `analyze_audio`, and where it runs

A new job kind in `src/jobs/worker.rs`, modelled directly on `media_checksum`,
which already does this exact shape of work: enqueue plus a periodic sweep for
anything missing a value, in capped batches (see migration 0039's header). Copy
the pattern; only the computation inside differs, and the sweep gains the
opt-in guard from §3.4.

**It gets its own container.** Production already runs three workers off one
image, split by `WORKER_JOB_TYPES` — that is what the split exists for, so this
is a compose block, not a new service or a new build:

| Existing container | Why not this one |
|---|---|
| `jobs` | `WORKER_MAX_CONCURRENT_JOBS: "1"`. A full-library backfill would hold the single slot for days and starve feed refreshes, stats rollups and billing behind it. |
| `assembler` | Audiobook assembly is the memory-heavy job and has 1 GB. A backfill competing with it is how both get slow. |

So: a fourth block, `image: audio2:prod`,
`WORKER_JOB_TYPES: "analyze_audio"`, **512m** and one job at a time. Decoding a
60-second window is tens of megabytes, so the limit is generous — and the
production host has 4 GB with no swap, shared with a CI runner, already carrying
four containers with 1 GB limits. Do not add a fifth at 1 GB without looking at
the real budget first.

> **The compose file on the server is not deployed by any workflow.**
> `docs/production-deployment.md` §4 says it outright: the backend workflow runs
> `docker compose up` against `/home/kornelko/audio2/docker-compose.yml`, and
> nothing copies that file from `deploy/production/`. **Adding the analyser
> block to the repo changes nothing on the host until someone scp's it, and no
> check detects the drift.** This is the step most likely to be forgotten when
> S3 ships, because everything else about the deploy is automatic.

**VPS or NUC is not a decision that has to be made now.** The queue lives in
Postgres and a worker is the same image with a different job-type filter, so
where it runs is a deployment detail. Start on the VPS at 512m; if it hurts,
move the same container to the NUC, which has cores and memory to spare. The
NUC's real cost is not performance — it is that it would need network access to
the production Postgres, which is a security decision, not a capacity one.

`ffmpeg` is already in the image (`backend/Dockerfile:63`), which covers
decoding and EBU R128 loudness. Tempo needs one more binary. See §3.7.

### 3.6 ShazamKit signatures, if they ever happen

`MUSIC_AI_STUDY.md` §10 asks whether ShazamKit custom-catalog signature
generation runs server-side at upload or client-side per device. **It is the
same question as this one and should get the same answer: server-side.** A file
already being fetched and decoded here can produce an audio-feature set and a
signature in one pass; scoping them separately would fetch the library twice.

Deferred until song recognition is actually scheduled. Recorded so the decision
is not made twice in opposite directions.

### 3.7 What the extractor may be

Server-only analysis settles most of the licensing question, but not all of it,
and the distinction is easy to get backwards.

**GPL-3 is triggered by distribution.** The backend is never handed to anyone —
the repository is internal and the product is sold as a service — so GPL-3
tooling carries no obligation here.

**AGPL-3 is triggered by network use.** That is exactly what a service is, so
AGPL tooling is *not* cleared by the same argument. This rules out the most
obvious candidate:

| Tool | Licence | Usable here |
|---|---|---|
| `ffmpeg` (default build) | LGPL-2.1+ | ✅ already in the image |
| `libebur128` | MIT | ✅ |
| `aubio` | GPL-3.0+ | ✅ not distributed |
| `bliss-audio` | GPL-3.0-only | ✅ by licence — but see §8 |
| **Essentia** | **AGPL-3.0** | ❌ network use is what AGPL §13 covers. A commercial licence is sold by UPF; that is a purchase decision, not a workaround. |

Recommended stack: **decode with `ffmpeg` to the analysis PCM contract, take
loudness from `ebur128`, take tempo from `aubio`, and compute spectral centroid,
RMS, dynamic range and onset rate directly from the PCM.** Those last four are a
few hundred lines over an FFT, not a framework dependency.

**Define the PCM contract explicitly** — mono, f32, a fixed sample rate — and
record it next to `analysis_version`. `bliss-audio` does this (22 050 Hz mono
f32) and it is the right idea to borrow: it makes the extractor's output
reproducible and testable rather than something that happens to work on one
machine.

---

## 4. Part C — smart playlists

### 4.1 The model

A smart playlist is **a saved query, an ordering, and a refresh policy** — not a
list of IDs. `music_playlists` stays exactly as it is for hand-made playlists
(0012); smart ones are a sibling.

```sql
CREATE TABLE music_smart_playlists (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    family_id   UUID        REFERENCES families (id) ON DELETE SET NULL,
    name        TEXT        NOT NULL,
    description TEXT,
    -- The saved query. Validated against a versioned schema on write, so a
    -- malformed or unknown rule can never reach evaluation.
    rule        JSONB       NOT NULL,
    rule_version SMALLINT   NOT NULL DEFAULT 1,
    -- 'dynamic' re-evaluates on every open; 'frozen' was materialised once.
    mode        TEXT        NOT NULL DEFAULT 'dynamic'
                            CHECK (mode IN ('dynamic', 'frozen')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

`family_id` follows the 0019 convention unchanged: NULL is private, set is
shared with the family.

### 4.2 The rule schema

One JSON object, deliberately small and closed. Anything not expressible here is
not expressible at all — which is what keeps it safe to accept from a language
model.

```jsonc
{
  "filters": {
    "bpm":        [95, 125],
    "energy":     [0.4, 0.7],
    "genres":     { "include": ["electronic"], "exclude": ["spoken"] },
    "artists":    { "exclude": ["..."] },
    "year":       [1990, 2005],
    "starred":    true,
    "min_rating": 4,
    "not_played_days": 365,
    "added_days": 30,
    "played_by_others_not_me": true
  },
  "limit":    { "kind": "duration", "minutes": 120 },
  "arc":      "warmup-sustain-cooldown",
  "sequence": { "max_per_artist": 2, "no_consecutive_artist": true },
  "sort":     "weighted-random"
}
```

Every filter is optional. `limit` is either `{ kind: "count", n }` or
`{ kind: "duration", minutes }`.

> **Known gap, found 2026-09-21 during `audio2-mac`'s M4 (`SMART_PLAYLISTS_PLAN.md`
> in that repo): `filters.genres` is an unvalidated `Vec<String>`.** The schema
> being closed keeps a client from inventing a *field* — it does nothing about
> an invented *value* in a field that is already free text. A rule with a genre
> string the library doesn't have passes `Rule::validate()` cleanly and then
> matches nothing in `smart_playlist_candidates`, so the caller gets a silent
> empty result with no reason given — the same "plausible, silent, wrong"
> failure mode two real bugs already took this session (§6, S6/day-filters and
> S4/energy-scale).
>
> Doesn't bite today because the only rule-producing paths are presets (a fixed
> set, already correct) and `POST /music/intent` (§5.2, whose own parser only
> ever emits genres it matched against the library's vocabulary in the first
> place). It bites the moment any client builds a rule *without* going through
> that parser — an on-device model, if one is ever revisited after
> `MUSIC_AI_STUDY.md`'s current no-go (see that doc's Spike A writeup), or a
> future Android on-device path. Fix belongs here, in `smart_playlist_candidates`
> or in `Rule::validate()`: check requested genres against the library's own
> vocabulary and either drop the unknown ones or reject the rule with the real
> list. Not fixed now — nothing currently shipping needs it — but it must land
> before any client is allowed to produce a rule outside this endpoint.

The five rules worth shipping as presets — each answers the "resurface the
forgotten" premise from §1 and needs no acoustic data at all, so they work
before Part B finishes:

| Preset | Rule |
|---|---|
| Forgotten favourites | starred, not played in a year |
| New in the library | added in 30 days, never played by me |
| Back in rotation | rating ≥ 4, not played in 60 days |
| What the family plays | played by other members, never by me |
| Genre station | genre filter, weight > 0.7, endless |

### 4.3 Evaluation

`POST /music/smart-playlists/{id}/resolve` → an ordered track list.

1. **Filter** — apply the rule, then subtract `banned` tracks and anything
   played too recently.
2. **Score** — `music_track_affinity.weight` × rule affinity.
3. **Sequence** — this is the step that makes the difference between a usable
   playlist and a random selection in the right tempo range:
   - no two consecutive tracks by the same artist; at most N per artist
   - follow the energy `arc` rather than a flat band. A two-hour Pilates set is
     not two hours of one intensity: roughly 10 minutes warming up, 90 minutes
     steady, 20 minutes coming down. That curve is deterministic code once
     tracks have numbers.
   - fill to the duration target greedily, then pick a final track that fits the
     remainder. Formally a knapsack; in practice this is enough.
4. **Weighted-random, not top-N.** Deterministic top-N returns the same playlist
   every time and gets stale in a week.

### 4.4 Sharing — three modes, named separately

A single "share" button that silently picks one of these would be the wrong
design. The question *does the rule travel, or the result?* has three legitimate
answers:

| Mode | What travels | Right for |
|---|---|---|
| **Share the rule** | the query | tools — but it evaluates differently for each member, because each has their own history. "Unplayed 80s rock" is not the same set for two people. |
| **Copy (fork)** | a frozen snapshot | "listen to what I made for you". The receiver can edit; the two never diverge because they never converge. |
| **Live shared** | one playlist, several editors | a shared context — the car, the kitchen, a party |

Hand-made playlists support copy and live. Smart playlists support rule and
copy (as "freeze and send"). Each is a distinct, explicitly labelled action.

---

## 5. Part D — natural language

The target interaction, in the user's words: *"I'm going for Pilates, make me a
two-hour playlist from my collection and start playing."*

### 5.1 The rule that matters

**The model never picks tracks. The model emits a rule.**

Input: one sentence, plus the library's available genres and value ranges (a few
hundred bytes). Output: a `rule` object from §4.2, validated against the schema.
Selection and sequencing are the deterministic code in §4.3.

Four reasons, and the first is sufficient on its own:

1. A model asked for tracks will invent tracks that are not in the library.
2. Twenty thousand tracks do not fit meaningfully in a context window — a
   retrieval-first design is required regardless of model size, as
   `MUSIC_AI_STUDY.md` §2 already concluded.
3. When the playlist comes out wrong, the rule is inspectable. A hallucinated
   tracklist is not debuggable.
4. Guided generation constrains the output to a typed struct, so a rule that
   fails validation is a bug to fix, not a runtime surprise.

This is compatible with the study's `searchTracks` tool-calling design — the
tool's parameters *are* this rule schema. Part B is what gives that tool an
`energy:` argument to take.

### 5.2 Where it runs

| Platform | Primary | Fallback |
|---|---|---|
| macOS, iOS | on-device `FoundationModels` (`LanguageModelSession` + `@Generable`) | server endpoint |
| tvOS, Android, web | server endpoint | — |

Backend provides `POST /music/intent` — text in, validated rule out — so every
platform reaches the same feature. Apple platforms should prefer on-device: it
costs nothing, works offline, and is what makes the privacy claim on the
marketing site true. Whether that endpoint calls a hosted model or a small local
one is deliberately left open; the contract is the rule schema either way.

**Nothing about the library is sent.** Only the sentence and the vocabulary of
available genres. That is a design constraint, not an implementation detail.

---

## 6. Implementation plan

Each milestone ends with: `cargo test` green, `cargo clippy` clean, verified
against the local stack (`docker compose up`), a `CHANGELOG.md` entry, and a
commit explaining the *why*. Per memory, work happens on `main` — no feature
branches while this is a one-person job — with a commit per milestone.

### ✅ S0 — `ended_reason` (done 2026-09-21)

Migration 0068 adds the nullable column and its CHECK, plus a partial index on
`(user_id, item_id, ended_reason)` for music rows that carry one — the only
rows S2's rollup ever reads. `report_sessions` accepts the optional field,
`record_sessions` persists it, and `derive_from_progress` leaves it NULL
because inference must not be mistaken for observation.

`normalize_ended_reason` deliberately does **not** follow
`normalize_device_kind`'s shape: an unrecognised device is harmlessly `other`,
but an unrecognised reason is dropped to NULL rather than bucketed. A wrong
bucket teaches the model something false; a missing one costs a data point.
Covered by two unit tests in `playback::tests`.

`docs/android-client-guide.md` documents the field, the four values, and the
instruction to instrument at the media-session layer rather than in the UI.

> **An earlier draft of this milestone also widened `device_kind` for `tvos`
> and `windows`. That was already done** — migrations 0064 and 0065, with the
> three Rust allowlists 0065's header names. The plan was written against
> migration 0020 and missed them. Nothing to do.

**Commit:** `feat(stats): record why a listening session ended`

### ✅ S1 — explicit feedback (done 2026-09-21)

Migration 0069 adds `music_track_feedback`, one row per (user, track) so moving
between `dislike` and `banned` is an upsert, with a partial index on the banned
rows because selection subtracts those before it scores anything.
`PUT`/`DELETE /music/tracks/{id}/feedback` and `GET /music/feedback?kind=`.

Verified against the local stack: upsert dislike→banned, `400` on an unknown
kind, `404` on a track the caller cannot see, `DELETE` idempotent (204 twice),
and — the claim that matters — **a banned track still returns 200 and still
appears in the track list.** Nothing about the track changes.

> **`AuthError::NotFound` means "no such user" and maps to 401**, not 404;
> `AuthError::ItemNotFound` is the 404 one. This milestone's endpoints were
> written against that distinction after the first version answered 401 for a
> track that does not exist.
>
> Several older handlers had the same confusion. **Fixed repo-wide in
> `eaac3ba`** (2026-09-21, by the user, across audiobooks, music and podcasts)
> — so the trap is gone rather than merely avoided here.

**Commit:** `feat(music): dislike and never-play feedback per user`

### ✅ S2 — affinity rollup (done 2026-09-21)

Migration 0070 plus `db::music::rebuild_affinity`, called from
`execute_stats_rollup`. Aggregation in SQL (the session log is large), the
weight itself in `music::affinity` (a product decision that will be retuned —
one copy in Rust beats two that can disagree). The skip boundary is bound into
the query from the same Rust constant rather than written twice.

Eleven unit tests pin the behaviour that matters: a skip lowers but never
zeroes, it recovers, repeats compound, a dislike outlasts a skip, and the
weight never leaves `[0, 1]`.

> **Found by running it against real sessions, not by a test.** The first
> version folded "played recently" into the stored weight, and a track played
> four times scored **0.203** against **0.273** for one skipped four times —
> the rest penalty had crushed the favourite. Two faults in one: rest is not
> taste, and a time-varying factor cannot live in a periodic rollup, where it
> freezes at whatever it was when the job last ran. Rest is now
> `affinity::recency_multiplier`, applied at selection time against
> `last_played_at`. A regression test pins the exact case.

**Commit:** `feat(music): per-user track affinity rollup`

### ✅ S3 — feature columns and `analyze_audio` (done 2026-09-21)

Migration for §3.2. `aubio` added to the image beside `ffmpeg`. Job kind copying
`media_checksum`'s enqueue-plus-sweep pattern, fetching from storage by range
where the format allows. The PCM contract from §3.7 documented next to
`analysis_version`. A fourth worker container per §3.5.

Verified against three real tracks: 133.8 / 107.3 / 113.1 BPM, −9.6 / −11.9 /
−13.2 LUFS, 12.5 / 13.6 / 18.3 dB dynamic range. Sane values, sane ordering.

> **Two bugs that only a real file could find, both silent:**
> 1. `aubio tempo` prints `114.57 bpm`, not a bare float. Parsed as a float,
>    every track in the library got no tempo at all.
> 2. `ebur128=framelog=quiet` is ffmpeg 6+. The runtime image is Debian
>    bookworm with **ffmpeg 5.1**, where the filter fails to initialise and
>    still prints a summary — of `0.0 LUFS`. The first full run "succeeded" and
>    wrote that constant for every track. The development Mac has ffmpeg 8, so
>    it worked there.
>
> Both are pinned by tests against the captured output, and a loudness of 0.0
> is now rejected outright: no real recording measures that, so it can only
> mean the toolchain failed.
>
> **A third trap, in the local stack:** `backend`, `assembler` and `analyzer`
> each build their own image from the same Dockerfile.
> `docker compose build backend` leaves the other two stale, and a worker whose
> image predates a migration crash-loops with *"migration N was previously
> applied but is missing in the resolved migrations"*. Use
> `docker compose build` with no service name.

**Commit:** `feat(music): audio feature extraction job`

### ✅ S4 — energy derivation (done 2026-09-21)

Materialised view or generated column computing normalised energy from the raw
measurements. Nightly refresh.

Verified by ear over 82 real tracks. Top of the scale: *Torn*, *She's So High*,
*It's All Been Done*, *Smooth*. Bottom: Queen's *Bijou*, *Teo Torriatte* and
*You Take My Breath Away*, and *I Can't Make You Love Me*. The quiet end is
genuinely quiet, which is the only test that means anything here.

> **Known limitation, not a bug.** Loudness carries 35 % of the weight, so the
> scale partly measures *mastering* rather than performance: the high end is
> all −6 to −9 LUFS, the low end −14 to −20. A quietly-mastered fast track
> scores lower than it should. The weights live in one place in migration 0072
> and changing them costs a `REFRESH`, not a re-measure — which is the whole
> point of storing raw measurements.

> **Throughput, measured:** ~1 s per track once the worker was fixed to drain
> at the speed of the work rather than one job per 30 s poll (see the
> Unreleased changelog entry). That is ~6 hours for 20 000 tracks at
> concurrency 1, against 167 before.

**Commit:** `feat(music): derive normalised track energy`

### ✅ S5 — the opt-in, the backfill and the sweep (done 2026-09-21)

A per-family setting, `PUT /family/settings/audio-analysis`. Enabling it
enqueues the whole visible library **ordered by play count, descending** (§3.4).
`GET` returns progress — measured, total, and whether a pass is running — so a
client can show a bar and a cancel. Disabling stops new work and keeps existing
rows.

The periodic sweep, with the opt-in guard: **only tracks belonging to families
that enabled it.**

Verified against a real 242-track library: enabling queued exactly the 163
unmeasured tracks, disabling kept every measurement, and re-enabling twice did
not duplicate the queue. Ordering confirmed by outcome — **every track with two
or more plays finished before any single-play track was claimed.**

> **The ranking was silently discarded twice before it worked.**
> 1. `INSERT ... ORDER BY` fixes physical insert order, not claim order.
>    `claim_next` orders by `scheduled_at`, and all 163 rows got the same
>    default timestamp, so the queue ran in arbitrary order.
> 2. Backdating by rank then ran it **exactly backwards**: ranking DESC gave
>    the most-played track the smallest offset into the past, so it was claimed
>    last. The window is ordered ASC so the best priority is backdated
>    furthest.
>
> Both failures look identical from outside — a queue that drains successfully,
> in the wrong order, with nothing logged.

The family-admin gate is `require_family_admin()`, the same helper guarding 23
other endpoints here. **The 403 was not exercised live** — the only non-admin
accounts are documented with production and canary passwords, which are not
credentials to use for a local test.

**Commit:** `feat(music): opt-in library analysis`

### ✅ S6 — smart playlists (done 2026-09-21)

`music_smart_playlists`, rule validation, CRUD, and `/resolve` with filtering,
scoring and sequencing. The five presets from §4.2.

Verified against the real 242-track library: a two-hour request returned
**117:52**, no artist twice in a row, none more than twice. Rules, validation
and sequencing carry 18 unit tests between them, including the arc shapes and
the duration budget.

> **`year` is not implemented.** §4.2 lists it, but `music_tracks` has no year
> column — not from the tags we read, not from MusicBrainz. It is absent from
> the schema rather than present and failing at query time. Adding it means a
> column and a backfill first.

> **The honest limit showed up immediately.** A Pilates-shaped rule (95–125 BPM,
> energy 0.3–0.8, two hours) returned 19 tracks and 84 minutes, because only 19
> tracks in the library match it. That is the ceiling being the library rather
> than the algorithm, exactly as §1 predicted — which is why the resolve
> response carries `candidates`, so a client can say *why* a playlist is short
> instead of looking broken.

**Commit:** `feat(music): smart playlists`

### ✅ S7 — sharing modes (done 2026-09-21)

`family_id` on smart playlists (rule sharing), `POST /{id}/freeze`
(freeze-to-copy), and `mark_source: false` for send-a-copy. Live-shared
hand-made playlists already existed — `music_playlists.family_id` from 0019
plus the visibility endpoint — so nothing new was needed for the third mode.

Verified: freezing produced a 12-track ordinary playlist and marked the source
`frozen` with `frozen_into` set.

**Commit:** `feat(music): smart playlist sharing modes`

### ✅ S8 — `POST /music/intent` (done 2026-09-21)

Text → validated rule. Returns the rule, not tracks; the caller resolves it.

Verified live: *"jedu na pilates na 2 hodiny"* → 120 minutes, 95–125 BPM,
warm-up/sustain/cool-down. *"gym session for 45 minutes"* → 45 minutes,
140–180 BPM, build. *"do the thing"* → refused. The full chain — sentence to
rule to resolved playlist — runs end to end.

> **What shipped is a deterministic baseline, not a model.** Six activity
> profiles, durations in English and Czech, genres matched against the
> library's own vocabulary, and the library qualifiers. Everything else is
> refused rather than guessed, because a confident playlist for a misread
> request cannot be debugged by the person who asked.
>
> This is deliberate and it settles §7 decision 4 for now: the **contract** is
> fixed (text in, validated rule out) before any model is chosen, so swapping
> one in behind it changes nothing downstream. It costs nothing and needs no
> network, which is what makes it a defensible default rather than a
> placeholder.

**Commit:** `feat(music): natural-language playlist intent endpoint`

---

## 7. Open decisions

1. **Is library analysis a paid feature?** It consumes real, measurable
   resources — a full fetch and decode of a library — on a product sold as a
   service, and `docs/family-billing-plan.md` already exists. If it is billed,
   the wording of the opt-in changes and S5 grows a check. Not a question this
   plan can answer.
2. **Does the production VPS plan meter total traffic, not just egress?** Many
   do. Worth knowing before ~200 GB moves through it, and it is the one thing
   that would force the NUC decision in §3.5 early rather than later.
3. ~~Buying the Essentia commercial licence.~~ **Settled 2026-09-21: no.**
   Essentia's only real advantage over `ffmpeg` + `aubio` + our own FFT code is
   its high-level classifiers — mood, danceability, genre — and §3.1 already
   says not to trust those; they were trained on small datasets and their
   accuracy is the part of AcousticBrainz that was criticised. Paying for a
   negotiated licence to obtain the one component we would discard is not worth
   an email. Revisit only if those classifiers ever become something we want.
4. **Whether `POST /music/intent` calls a hosted model at all.** Tied to
   `MUSIC_AI_STUDY.md` §9 row 4 and its recurring-cost question. The endpoint
   can ship backed by a local model and be swapped later; the contract does not
   change.
5. **Preset visibility.** Whether the five presets are rows every user gets,
   or code-level built-ins. Rows are editable and forkable; built-ins are one
   less migration.

---

## 8. Prior art

Checked 2026-09-21, so the ground already covered is not covered again.

| Project | Has | Lacks |
|---|---|---|
| **Navidrome** | Real smart playlists — `.nsp` JSON rule files, `all`/`any`, operators `is`/`gt`/`inTheLast`/`inTheRange`, fields `playCount`, `rating`, `starred`, `lastPlayed`, `dateAdded`, `genre`, `year`, **`bpm`** | Reads `bpm` from the file tag, never computes it, so it is empty for most libraries. No skip measurement, no energy, no decay |
| **Lyrion** (ex-LMS) | The *Music Similarity* plugin runs Essentia locally over the library and serves similarity mixes; *Don't Stop The Music* extends the queue | Weaker rule engine, no skip measurement |
| **beets** | `xtractor` runs Essentia locally, `acousticbrainz` fetches dumps, `smartplaylist` emits m3u | Not a server or a player |
| **Jellyfin** | Instant Mix over metadata, a Smart Playlist plugin | No acoustic analysis |
| **Funkwhale** | Radios — similar artist, tags, less-listened | No analysis, no rules |
| **Plex** | Sonic Analysis: really does compute acoustic features and builds Sonically Similar / Sonic Adventures on them | Closed |

**Nobody measures skips.** Navidrome has play counts, the Subsonic protocol has
`scrobble`, stars and ratings — but no self-hosted project records *why a track
ended*, and none has a decaying preference weight or the dislike/ban
distinction. Part A is the genuinely novel piece here, and it is also the
cheapest.

**Worth borrowing:** Navidrome's `.nsp` rule schema, rather than inventing the
§4.2 format from nothing. It is proven and documented, and `audio2` already
imports Navidrome playlists through Connected Servers — so matching it would
make smart-playlist import fall out for free instead of being designed later.

**`bliss-audio` was evaluated and rejected.** Actively maintained (0.13.0,
August 2026; 164 stars, 6 open issues), Rust, and GPL-3 is not a problem for a
service. It is rejected on shape, not licence: its `Analysis` is an opaque `f32`
vector for computing distance, and it exposes no named, interpretable features.
It answers "play more like this one," not "95–125 BPM at medium energy." If
sonic-similarity radio is ever wanted, revisit it then — as a separate process,
since it is GPL-3 and the clients are distributed.
