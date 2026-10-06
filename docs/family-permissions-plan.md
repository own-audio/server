# Family member permissions and age brackets

Driven by two things at once: Google Play's Families Policy (the Android app is
declared for children/families — see `audio2-android-book/PLAY_COMPLIANCE.md`),
and a product gap. A parent has no way to say what a child may do.

The compliance half is not optional. An app declared for children that lets any
member upload arbitrary content and generate AI narration is unmoderated UGC,
which is the single most likely reason review fails.

**The rule that shapes everything below: enforcement is server-side.** Hiding a
button is UX. The backend must refuse the request, or it is not a control.

## What exists today

`family_members` (migration 0018) is:

```
family_id, user_id (UNIQUE — one family per user), role, display_label, joined_at
role CHECK IN ('family_admin', 'member')
```

`MemberResponse` exposes `role`, `is_active`, `pending`, `avatar_url`,
`joined_at`. There is no age and no per-member permission anywhere, and every
member can reach every upload and generation endpoint.

## Schema — migration 0056

```sql
ALTER TABLE family_members
    ADD COLUMN age_bracket  TEXT    NOT NULL DEFAULT 'adult'
        CHECK (age_bracket IN ('adult', 'teen', 'child')),
    ADD COLUMN can_upload   BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN can_generate BOOLEAN NOT NULL DEFAULT TRUE;

-- A child never produces content in this app. This is the compliance
-- invariant, so it is a constraint rather than a default an admin can
-- silently undo.
ALTER TABLE family_members ADD CONSTRAINT family_members_child_no_ugc
    CHECK (age_bracket <> 'child' OR (can_upload = FALSE AND can_generate = FALSE));
```

Defaults are deliberate: every existing row becomes `adult / true / true`, so
migrating changes nobody's behaviour. New restrictions only ever arrive by an
admin choosing them.

**Store a bracket, never a birthdate.** A date of birth is itself a child's
personal data and drags in obligations a bracket does not. The bracket is set by
the *parent* when inviting, which is also the parental-consent shape.

Invite-time brackets are deliberately **not** in 0056. Nothing would read them
yet, and dead schema reads as done. A provisioned account has no auth identity
until it is claimed, so nobody can sign in as it — there is no window in which
a new child member could upload before the admin sets the bracket. Two calls
are safe, and the columns land with the admin UI (P4).

## API

**`MemberResponse`** gains `age_bracket`, `can_upload`, `can_generate`.

**`FamilyResponse`** gains `my_can_upload`, `my_can_generate`, alongside the
existing `my_role` / `my_user_id`. Clients must not have to find themselves in
`members[]` to know what they may do.

**`PUT /api/v1/family/members/{user_id}`** — the existing `update_member`
endpoint, extended rather than duplicated. `family_admin` only.
Body gains `{ age_bracket?, can_upload?, can_generate? }` alongside the
existing `role?` and `display_label?`.

Guard rails, all 400/403 rather than silent no-ops:
- A member cannot change their own permissions, admin or not.
- The last `family_admin` cannot be demoted — the family must not be lockable.
- Setting `age_bracket = 'child'` forces both flags false (mirrors the CHECK).

## Enforcement points

Every one of these must check the caller's `can_upload` / `can_generate` and
return **403** otherwise:

| Route | Handler | Gate |
|---|---|---|
| `POST /audiobooks/upload` | `audiobooks::upload_book` | `can_upload` |
| `POST /audiobooks/from-uploads` | `audiobooks::create_book_from_uploads` | `can_upload` |
| `POST /audiobooks/{id}/upload-file` | `audiobooks::upload_file` | `can_upload` |
| `POST /audiobooks/{id}/upload-cover` | `audiobooks::upload_cover` | `can_upload` |
| `POST /audiobook-gen/jobs` | `audiobook_gen::create_job` | `can_generate` |

`/audiobook-gen/quote` and `/estimate` stay open — they are read-only pricing
and leak nothing. Blocking them only produces a worse error later.

Implement as one extractor/helper (`require_can_upload`, `require_can_generate`)
next to the existing family-role checks, so a new upload route cannot forget it.

## Clients

Order matters: Android first, because Play review is the deadline.

**Android** (`audio2-android-book`)
- Hide the Generate entry point (`AudiobookApp.kt`, `onGenerate` → `Destinations.GENERATE`) unless `my_can_generate`.
- Hide upload affordances unless `my_can_upload` (`UploadController`, library add).
- Family screen: admin can set bracket + permissions per member.
- A 403 from the server must still read as a clear message, not a crash — the client gate can be stale.

**iOS / macOS** — same three, after Android.

## Tests

- Migration: existing rows land `adult/true/true`; the child CHECK rejects `child + can_upload`.
- Each of the five routes: 403 for a member without the flag, 200 for one with it.
- `PATCH`: admin succeeds; member is 403; self-edit is 403; last-admin demotion is 400; `child` forces both flags false.
- Claim: an invite carrying `child` produces a member with both flags false.
- Android: `canGenerate == false` hides the entry point; a 403 surfaces the server message.

## Phases

- [x] **P1 — schema + API.** ✅ 2026-08-27. Migration 0056; `age_bracket` /
  `can_upload` / `can_generate` on `family_members` with the child CHECK;
  `Membership` struct replacing the `(Uuid, String)` tuple from
  `find_membership`/`ensure_membership`; `MemberResponse` and `FamilyResponse`
  (`my_can_upload`/`my_can_generate`) extended; `update_member` extended with
  the guard rails.
- [x] **P2 — enforcement.** ✅ 2026-08-27. `FamilyContext` carries the flags and
  exposes `require_can_upload` / `require_can_generate`; all five routes call
  one of them. Putting the gate on the extractor — which every one of those
  handlers already takes — is what stops a new upload route shipping without it.
- [x] **P3 — Android gating.** ✅ 2026-08-27. `FamilyDto` carries
  `my_can_upload` / `my_can_generate` (defaulting permissive so an older server
  behaves as before); `LibraryScreen` hides the add menu, its two items and the
  empty-state buttons accordingly. The share-intent jump to Add book was the
  non-obvious one — it bypasses the library UI entirely, so it now waits for a
  resolved answer instead of assuming permission, and drops the share when the
  account may not upload. `null` means "not fetched yet", which affordances
  treat as permissive (no flicker) and that one-shot navigation does not.
- [ ] **P4 — Android admin UI.** Family screen editing.
- [ ] **P5 — iOS + macOS.** Gating, then admin UI.

### Decisions taken during P1/P2

- **Family admins always pass the gate.** They hand the permission out; refusing
  them would only let a family lock itself out of its own library.
  `effective_can_upload()` is the single definition, used by both the guard and
  the response, so what a client is told and what the server enforces cannot
  drift.
- **Nobody edits their own permissions**, admin included — a restriction you can
  lift from yourself is advisory. Role and label keep their previous behaviour.
- **Moving a member to `child` clears both flags** rather than erroring, so an
  admin need not send three fields to restrict one member. Asking outright for
  a child to upload is refused, not silently ignored.
- `resolve_member_permissions` is a pure function so the child invariant is
  unit-testable without a database, matching how `validate_topup_amount` is
  tested in the same module.

## Not in scope

- Per-member playback restrictions (content ratings, explicit filtering, time
  limits). Different problem, different data.
- Multiple families per user — `family_members.user_id` is UNIQUE and v1 keeps it.
- Verifiable parental consent flows beyond parent-provisioned accounts. The
  `claim` invite path already avoids collecting a child's email at all, which is
  what keeps COPPA out of scope.
