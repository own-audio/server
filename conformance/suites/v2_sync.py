# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 sync essentials (completion plan phase 3): `kinds=` on the change feed, the ids-only snapshot, the `member` kind, file facts on records (`path`, `sha256`, `relative_path`), renames, grants, policies and sharing in the change feed, and continue listening in one request (`GET /me/progress` with several kinds, `unfinished`, `include=`).

The strongest check is the first: a copy built from a snapshot and the change feed must equal a
fresh snapshot, for an owner and for another member, after uploads, a rename, sharing, a grant,
a deletion and listening.
"""
from __future__ import annotations

import json
import re
import urllib.parse

from core import Ctx

NAME = "v2_sync"
PW = "V2SyncTest123!"
PROBLEM = "application/problem+json"


def _problem(answer, status: int, code: str | None = None) -> bool:
    got, headers, body = answer
    if got != status or not headers.get("content-type", "").startswith(PROBLEM):
        return False
    try:
        doc = json.loads(body)
    except ValueError:
        return False
    return doc.get("status") == status and (code is None or doc.get("code") == code)


def _lines(raw: bytes) -> list[dict]:
    return [json.loads(line) for line in raw.decode().splitlines() if line.strip()]


# Signed links carry a signature that may differ between two readings; compare where they point.
_SIGNED = re.compile(r"\?.*$")


def _norm(value):
    if isinstance(value, dict):
        return {k: _norm(v) for k, v in value.items()}
    if isinstance(value, list):
        return [_norm(v) for v in value]
    if isinstance(value, str) and value.startswith(("http://", "https://", "/api/")):
        return _SIGNED.sub("", value)
    return value


class Copy:
    """What a client keeps: a snapshot, then the change feed applied to it."""

    def __init__(self, ctx: Ctx, token: str, kinds: str | None = None) -> None:
        self.ctx, self.token, self.kinds = ctx, token, kinds
        self.records: dict[tuple[str, str], dict] = {}
        self.cursor = ""
        self.resets = 0
        self.snapshot()

    def snapshot(self) -> None:
        q = f"?kinds={self.kinds}" if self.kinds else ""
        st, _h, raw = self.ctx.request("GET", f"/api/v2/sync/snapshot{q}", self.token)
        lines = _lines(raw) if st == 200 else []
        assert lines and lines[-1].get("type") == "end", f"snapshot {st}"
        self.records = {(l["kind"], l["id"]): l["record"] for l in lines if l.get("type") == "record"}
        self.cursor = lines[-1]["cursor"]

    def pull(self, limit: int = 500) -> list[dict]:
        """Applies everything new; answers the changes read."""
        seen: list[dict] = []
        for _ in range(200):
            q = {"cursor": self.cursor, "limit": str(limit)}
            if self.kinds:
                q["kinds"] = self.kinds
            st, _h, raw = self.ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode(q), self.token)
            assert st == 200, f"changes {st} {raw[:200]!r}"
            page = json.loads(raw)
            if page["reset"]:
                self.resets += 1
                self.snapshot()
                continue
            for c in page["changes"]:
                seen.append(c)
                key = (c["kind"], c["id"])
                if c["op"] == "upsert":
                    self.records[key] = c["record"]
                else:
                    self.records.pop(key, None)
            self.cursor = page["cursor"]
            if not page["has_more"]:
                break
        return seen


def _same_as_snapshot(ctx: Ctx, copy: Copy, label: str) -> None:
    """The copy against a fresh snapshot, both brought up to now so that background work
    between the two readings (checksums, fingerprints) lands on both."""
    fresh = Copy(ctx, copy.token, copy.kinds)
    copy.pull()
    fresh.pull()
    mine = {k: _norm(v) for k, v in copy.records.items()}
    theirs = {k: _norm(v) for k, v in fresh.records.items()}
    missing = sorted(set(theirs) - set(mine))
    extra = sorted(set(mine) - set(theirs))
    differ = sorted(k for k in set(mine) & set(theirs) if mine[k] != theirs[k])
    ctx.check(label, not (missing or extra or differ),
              f"missing {missing[:4]}, extra {extra[:4]}, differ {[(k, mine[k], theirs[k]) for k in differ[:1]]}"[:600])


def _tone() -> bytes:
    """Real audio, so the fingerprint job does not set the suite's songs aside (quarantine) in
    the middle of the run, as it does random bytes."""
    return Ctx.tone()


def _upload(ctx: Ctx, token: str, kind: str, name: str) -> str:
    audio, ctype = (_tone(), "audio/wav") if kind == "track" else (ctx.audio(), "audio/mpeg")
    if kind == "track":
        name = name.rsplit(".", 1)[0] + ".wav"
    made = ctx.call("POST", "/api/v2/uploads", token,
                    {"kind": kind, "filename": name, "content_type": ctype, "size_bytes": len(audio)}, expect=(201,))
    ctx.put_bytes(made["url"], audio, made["headers"]["Content-Type"])
    return made["upload_id"]


def run(ctx: Ctx) -> None:
    sfx = ctx.sfx
    owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    kid_id, _ke, kid = ctx.make_user("kid", PW)

    problems(ctx, owner)
    owner_copy = Copy(ctx, owner)
    kid_copy = Copy(ctx, kid)

    made = records_with_file_facts(ctx, owner)
    renamed(ctx, owner, made["track"])
    shared(ctx, owner, kid, kid_id, made)
    granted(ctx, owner, kid, kid_id, made["track"])
    policy(ctx, owner, kid, kid_id)
    checksums(ctx, owner, made)
    continue_listening(ctx, owner, kid, made)

    # Some of it goes again, and someone who shared a song deletes their account.
    ctx.call("DELETE", f"/api/v1/music/tracks/{made['other_track']}", owner, expect=(200, 204))
    members(ctx, owner, owner_id, kid, kid_id)
    _same_as_snapshot(ctx, owner_copy, "the owner's snapshot and changes reproduce the library, file facts included")
    _same_as_snapshot(ctx, kid_copy, "another member's snapshot and changes reproduce what they see")

    kinds_filter(ctx, owner, made)
    ids_only(ctx, owner)


def problems(ctx: Ctx, tok: str) -> None:
    check = ctx.check
    check("changes: an unknown kind is 400 invalid_kind",
          _problem(ctx.request("GET", "/api/v2/sync/changes?kinds=track,teleport", tok), 400, "invalid_kind"))
    check("changes: a state kind that does not exist is 400 invalid_kind",
          _problem(ctx.request("GET", "/api/v2/sync/changes?kinds=rating.book", tok), 400, "invalid_kind"))
    check("snapshot: fields other than ids is 400 invalid_fields",
          _problem(ctx.request("GET", "/api/v2/sync/snapshot?fields=everything", tok), 400, "invalid_fields"))
    check("snapshot: an unknown kind is 400 invalid_kind",
          _problem(ctx.request("GET", "/api/v2/sync/snapshot?kinds=progress.book", tok), 400, "invalid_kind"))
    check("progress: a kind without progress is 400 unsupported_kind",
          _problem(ctx.request("GET", "/api/v2/me/progress?kind=book,album", tok), 400, "unsupported_kind"))
    check("progress: an unknown include is 400 invalid_include",
          _problem(ctx.request("GET", "/api/v2/me/progress?kind=book&include=items,genres", tok), 400, "invalid_include"))
    st, _h, _b = ctx.request("GET", "/api/v2/sync/changes?kinds=member,progress.episode", tok)
    check("changes: kinds of state by name are taken", st == 200, str(st))


def records_with_file_facts(ctx: Ctx, tok: str) -> dict:
    """Creates through v2 and checks `path`, `sha256`, `relative_path` in the answers."""
    check, sfx = ctx.check, ctx.sfx
    path = f"Music/Sync Facts {sfx}/Disc/01 First.wav"
    track = ctx.call("POST", "/api/v2/tracks", tok, {"upload_id": _upload(ctx, tok, "track", "01.mp3"), "filename": "01 First.wav",
                                                      "title": f"Facts Song {sfx}", "artist": f"Facts Band {sfx}",
                                                      "album": f"Facts Album {sfx}", "path": path, "visibility": "family"},
                     expect=(201,))
    check("POST /tracks answers the path it was given", track.get("path") == path, str(track.get("path")))
    check("…and a sha256 field (null until the server has read the file)", "sha256" in track)
    other = ctx.call("POST", "/api/v2/tracks", tok, {"upload_id": _upload(ctx, tok, "track", "02.mp3"), "filename": "02 Second.mp3",
                                                      "title": f"Facts Other {sfx}", "artist": f"Facts Band {sfx}",
                                                      "album": f"Facts Album {sfx}", "visibility": "family"}, expect=(201,))
    check("without a path a track gets its default one", str(other.get("path", "")).startswith("Music/"), str(other.get("path")))
    book_path = f"Audiobooks/Facts Writer {sfx}/Facts Book"
    book = ctx.call("POST", "/api/v2/books", tok, {
        "title": f"Facts Book {sfx}", "author": f"Facts Writer {sfx}", "visibility": "family", "path": book_path,
        "files": [{"upload_id": _upload(ctx, tok, "book_file", "b2.mp3"), "relative_path": "CD1/02 Two.mp3"},
                  {"upload_id": _upload(ctx, tok, "book_file", "b1.mp3"), "relative_path": "CD1/01 One.mp3"}]}, expect=(201,))
    files = book.get("files") or []
    check("POST /books answers its folder and each file's relative path, in play order",
          book.get("path") == book_path and [f.get("relative_path") for f in files] == ["CD1/01 One.mp3", "CD1/02 Two.mp3"]
          and all("sha256" in f for f in files), json.dumps(book)[:300])
    show = ctx.call("POST", "/api/v2/podcasts", tok, {"title": f"Facts Show {sfx}", "visibility": "private"}, expect=(200, 201))
    episode = ctx.call("POST", f"/api/v2/podcasts/{show['id']}/episodes", tok, {
        "upload_id": _upload(ctx, tok, "episode", "e.mp3"), "guid": f"facts-{sfx}", "title": f"Facts Episode {sfx}"}, expect=(200, 201))
    check("an uploaded episode answers where its stored copy sits",
          str(episode.get("path") or "").startswith("Podcasts/") and "sha256" in episode, str(episode.get("path")))
    got = ctx.call("GET", f"/api/v2/tracks/{track['id']}", tok)
    check("GET /tracks/{id} carries the path too", got.get("path") == path)
    return {"track": track["id"], "other_track": other["id"], "album": track.get("album_id"), "book": book["id"],
            "podcast": show["id"], "episode": episode["id"], "track_path": path}


def renamed(ctx: Ctx, tok: str, track_id: str) -> None:
    """A rename in the own.audio folder (here: v1's organise, which moves files to their default
    paths) is one change of the track, with its new path."""
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?kinds=track", tok)
    cursor = json.loads(raw)["cursor"]
    moved = ctx.call("POST", "/api/v1/sync/paths/organise", tok, {"kind": "music", "ids": [track_id]})
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "track"}), tok)
    page = json.loads(raw) if st == 200 else {}
    mine = [c for c in page.get("changes", []) if c["id"] == track_id]
    new_path = moved[0].get("to") if moved else None
    ctx.check("a rename is one change of the track, with the new path",
              bool(moved) and len(mine) == 1 and mine[0]["op"] == "upsert" and mine[0]["record"]["path"] == new_path,
              f"{moved} {mine}"[:300])


def shared(ctx: Ctx, owner: str, kid: str, kid_id: str, made: dict) -> None:
    """Sharing an item makes what it refers to visible too: the album and the band with a track,
    the episodes with a show."""
    sfx = ctx.sfx
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes", kid)
    cursor = json.loads(raw)["cursor"]
    private = ctx.call("POST", "/api/v2/tracks", owner, {"upload_id": _upload(ctx, owner, "track", "p.mp3"), "filename": "p.mp3",
                                                          "title": f"Later Song {sfx}", "artist": f"Later Band {sfx}",
                                                          "album": f"Later Album {sfx}", "visibility": "private"}, expect=(201,))
    ctx.call("PUT", f"/api/v2/tracks/{private['id']}/visibility", owner, {"visibility": "family"}, expect=(200,))
    ctx.call("PUT", f"/api/v2/podcasts/{made['podcast']}/visibility", owner, {"visibility": "family"}, expect=(200,))
    got: dict = {}
    for _ in range(50):
        st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor}), kid)
        page = json.loads(raw)
        for c in page["changes"]:
            got[(c["kind"], c["id"])] = c
        cursor = page["cursor"]
        if not page["has_more"]:
            break
    ok = lambda k: got.get(k, {}).get("op") == "upsert"  # noqa: E731
    band = [c for (k, _i), c in got.items() if k == "contributor" and c.get("record", {}).get("name") == f"Later Band {sfx}"]
    ctx.check("a track shared later reaches a member with its album and its band",
              ok(("track", private["id"])) and ok(("album", private.get("album_id"))) and bool(band) and band[0]["op"] == "upsert",
              str(sorted(got))[:300])
    ctx.check("a show shared later reaches a member with its episodes",
              ok(("podcast", made["podcast"])) and ok(("episode", made["episode"])), str(sorted(got))[:300])
    made["shared_later"] = private["id"]


def granted(ctx: Ctx, owner: str, kid: str, kid_id: str, track_id: str) -> None:
    """A per-item grant changes what one member sees: that item arrives as a change, for them."""
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?kinds=track,album", kid)
    cursor = json.loads(raw)["cursor"]
    ctx.call("PUT", f"/api/v1/family/members/{kid_id}/grants", owner, {"media_kind": "music", "deny": [track_id]}, expect=(200, 204))
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "track,album"}), kid)
    page = json.loads(raw)
    ctx.check("a grant that hides a track is a removal of it, for that member",
              any(c["kind"] == "track" and c["id"] == track_id and c["op"] == "delete" for c in page["changes"]), str(page)[:300])
    ctx.call("PUT", f"/api/v1/family/members/{kid_id}/grants", owner, {"media_kind": "music"}, expect=(200, 204))
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": page["cursor"], "kinds": "track,album"}), kid)
    back = json.loads(raw)
    ctx.check("…and lifting it sends the track again",
              any(c["kind"] == "track" and c["id"] == track_id and c["op"] == "upsert" for c in back["changes"]), str(back)[:300])
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "track"}), owner)
    ctx.check("another member's grant does not reach the owner's feed",
              not any(c["id"] == track_id for c in json.loads(raw)["changes"]))


def policy(ctx: Ctx, owner: str, kid: str, kid_id: str) -> None:
    """A whole kind hidden or shown for a member: that member starts over from a snapshot."""
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?kinds=track", kid)
    cursor = json.loads(raw)["cursor"]
    ctx.call("PUT", f"/api/v1/family/members/{kid_id}/policy", owner, {"media_kind": "podcast", "policy": "deny_all"}, expect=(200, 204))
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "track"}), kid)
    ctx.check("a changed media policy is a reset for that member, whatever kinds they read", json.loads(raw).get("reset") is True)
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor}), owner)
    ctx.check("…and not for anyone else", json.loads(raw).get("reset") is False)
    ctx.call("PUT", f"/api/v1/family/members/{kid_id}/policy", owner, {"media_kind": "podcast", "policy": "allow_all"}, expect=(200, 204))


def checksums(ctx: Ctx, tok: str, made: dict) -> None:
    """The server reads each file once after the upload; its checksum then reaches the record."""
    def has() -> bool:
        st, _h, raw = ctx.request("GET", f"/api/v2/tracks/{made['track']}", tok)
        return st == 200 and bool(json.loads(raw).get("sha256"))
    if not ctx.wait_until(has, timeout=60):
        ctx.skip("a computed checksum reaches the change feed", "the checksum job did not run within a minute")
        return
    st, _h, raw = ctx.request("GET", f"/api/v2/tracks/{made['track']}", tok)
    sha = json.loads(raw)["sha256"]
    ctx.check("a track's sha256 is 64 hex characters", bool(re.fullmatch(r"[0-9a-f]{64}", sha)), sha)
    st, _h, raw = ctx.request("GET", f"/api/v2/books/{made['book']}", tok)
    ctx.wait_until(lambda: all(f.get("sha256") for f in json.loads(ctx.request("GET", f"/api/v2/books/{made['book']}", tok)[2])["files"]), timeout=60)
    book = json.loads(ctx.request("GET", f"/api/v2/books/{made['book']}", tok)[2])
    ctx.check("each book file gets its sha256", all(re.fullmatch(r"[0-9a-f]{64}", f.get("sha256") or "") for f in book["files"]))


def continue_listening(ctx: Ctx, owner: str, kid: str, made: dict) -> None:
    check = ctx.check
    sfx = ctx.sfx
    ctx.call("PUT", f"/api/v2/me/progress/track/{made['track']}", owner, {"position_ms": 1000}, expect=(200,))
    ctx.call("PUT", f"/api/v2/me/progress/book/{made['book']}", owner, {"position_ms": 2000}, expect=(200,))
    ctx.call("PUT", f"/api/v2/me/progress/episode/{made['episode']}", owner, {"position_ms": 3000}, expect=(200,))
    ctx.call("PUT", f"/api/v2/me/progress/track/{made['shared_later']}", owner, {"position_ms": 1, "finished": True}, expect=(200,))
    page = ctx.call("GET", "/api/v2/me/progress?kind=track,book,episode&include=items,podcasts,contributors,albums", owner)
    items = page["items"]
    stamps = [i["updated_at"] for i in items]
    check("several kinds in one list, most recent first",
          {i["kind"] for i in items} >= {"track", "book", "episode"} and stamps == sorted(stamps, reverse=True), str(items)[:300])
    inc = page.get("included") or {}
    for kind, key in (("track", "tracks"), ("book", "books"), ("episode", "episodes")):
        listed = [i["id"] for i in items if i["kind"] == kind]
        got = [r["id"] for r in inc.get(key, [])]
        check(f"include=items embeds each listed {kind} once", sorted(got) == sorted(set(listed)), f"{got} vs {listed}")
    pods = [p["id"] for p in inc.get("podcasts", [])]
    check("include=podcasts embeds the episodes' shows once", pods.count(made["podcast"]) == 1, str(pods))
    check("include=albums and contributors embed what the items name",
          any(a["id"] == made["album"] for a in inc.get("albums", [])) and len(inc.get("contributors", [])) > 0)
    check("a book embedded through include= is the list shape, without its files",
          all("files" not in b for b in inc.get("books", [])))
    plain = ctx.call("GET", "/api/v2/me/progress?kind=book", owner)
    check("without include= nothing is embedded", "included" not in plain)

    unfinished = ctx.call("GET", "/api/v2/me/progress?kind=track,book,episode&unfinished=true", owner)["items"]
    check("unfinished=true leaves out what is finished",
          made["shared_later"] not in {i["id"] for i in unfinished} and made["track"] in {i["id"] for i in unfinished})

    whole = [(i["kind"], i["id"]) for i in ctx.call("GET", "/api/v2/me/progress?kind=track,book,episode&limit=500", owner)["items"]]
    walked, cursor = [], None
    for _ in range(50):
        q = {"kind": "track,book,episode", "limit": "1", "include": "items"}
        if cursor:
            q["cursor"] = cursor
        p = ctx.call("GET", "/api/v2/me/progress?" + urllib.parse.urlencode(q), owner)
        walked += [(i["kind"], i["id"]) for i in p["items"]]
        check_one = len(p.get("included", {}).get("tracks", []) + p.get("included", {}).get("books", [])
                        + p.get("included", {}).get("episodes", [])) == len(p["items"])
        if not check_one:
            check("each page embeds its own items only", False, json.dumps(p)[:200])
            break
        cursor = p.get("next")
        if not cursor:
            break
    check("paged one by one: the same items, in the same order, none twice", walked == whole and len(set(walked)) == len(walked),
          f"{len(walked)} vs {len(whole)}")

    # Only what the listener can see: a member's progress on a book the owner then keeps private.
    ctx.call("PUT", f"/api/v2/me/progress/book/{made['book']}", kid, {"position_ms": 500}, expect=(200,))
    before = ctx.call("GET", "/api/v2/me/progress?kind=book,episode&include=items", kid)
    ctx.call("PUT", f"/api/v2/books/{made['book']}/visibility", owner, {"visibility": "private"}, expect=(200,))
    after = ctx.call("GET", "/api/v2/me/progress?kind=book,episode&include=items", kid)
    check("progress on what is no longer visible leaves the list and its embedded records",
          made["book"] in {i["id"] for i in before["items"]}
          and made["book"] not in {i["id"] for i in after["items"]}
          and made["book"] not in {b["id"] for b in (after.get("included") or {}).get("books", [])})
    ctx.call("PUT", f"/api/v2/books/{made['book']}/visibility", owner, {"visibility": "family"}, expect=(200,))
    _ = sfx


def kinds_filter(ctx: Ctx, tok: str, made: dict) -> None:
    """`kinds=` keeps only those kinds, filtered by the server, and pages without loss."""
    sfx = ctx.sfx
    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes", tok)
    start = json.loads(raw)["cursor"]
    for i in range(5):
        t = ctx.call("POST", "/api/v2/tracks", tok, {"upload_id": _upload(ctx, tok, "track", f"k{i}.mp3"), "filename": f"k{i}.mp3",
                                                      "title": f"Kinds Song {sfx} {i}", "artist": f"Kinds Band {sfx}",
                                                      "album": f"Kinds Album {sfx}"}, expect=(201,))
        ctx.request("PUT", f"/api/v2/me/favourites/track/{t['id']}", tok)
        ctx.call("PUT", f"/api/v2/me/progress/track/{t['id']}", tok, {"position_ms": 10 * (i + 1)}, expect=(200,))

    def read(kinds: str | None, limit: int) -> list[tuple[str, str]]:
        out, cursor = [], start
        for _ in range(500):
            q = {"cursor": cursor, "limit": str(limit)}
            if kinds:
                q["kinds"] = kinds
            page = json.loads(ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode(q), tok)[2])
            out += [(c["kind"], c["id"]) for c in page["changes"]]
            cursor = page["cursor"]
            if not page["has_more"]:
                break
        return out

    everything = read(None, 1000)
    tracks = read("track", 2)
    state = read("favourite,progress.track", 2)
    ctx.check("kinds=track sends tracks only", bool(tracks) and all(k == "track" for k, _ in tracks), str(tracks[:5]))
    ctx.check("…the same tracks as the unfiltered feed, paged by 2 without loss",
              set(tracks) == {x for x in everything if x[0] == "track"}, f"{len(set(tracks))} vs {len({x for x in everything if x[0] == 'track'})}")
    ctx.check("kinds=favourite,progress.track: every favourite and the tracks' progress, nothing else",
              set(state) == {x for x in everything if x[0].startswith("favourite.") or x[0] == "progress.track"} and len(state) >= 10,
              str(state[:4]))
    _ = made


def ids_only(ctx: Ctx, tok: str) -> None:
    st, headers, raw = ctx.request("GET", "/api/v2/sync/snapshot", tok)
    full = _lines(raw)
    st2, headers2, raw2 = ctx.request("GET", "/api/v2/sync/snapshot?fields=ids", tok)
    ids = _lines(raw2)
    full_keys = {(l["kind"], l["id"]) for l in full if l.get("type") == "record"}
    id_keys = [(l["kind"], l["id"]) for l in ids if l.get("type") == "id"]
    ctx.check("fields=ids: begin, id lines without records, end",
              st2 == 200 and headers2.get("content-type", "").startswith("application/x-ndjson") and ids[0]["type"] == "begin"
              and ids[-1]["type"] == "end" and all("record" not in l for l in ids), f"{st2} {len(ids)}")
    ctx.check("the ids-only snapshot lists the same items as the full one", set(id_keys) == full_keys and len(id_keys) == len(set(id_keys)),
              f"{len(set(id_keys))} vs {len(full_keys)}; only in ids {sorted(set(id_keys) - full_keys)[:3]}, only in full {sorted(full_keys - set(id_keys))[:3]}")
    ctx.check("it is much smaller", len(raw2) * 3 < len(raw), f"{len(raw2)} vs {len(raw)} bytes")
    tracks = [k for k in id_keys if k[0] == "track"]
    if len(tracks) >= 2:
        after = tracks[0]
        st, _h, rest = ctx.request("GET", f"/api/v2/sync/snapshot?fields=ids&kinds=track&after=track:{after[1]}&cursor={ids[0]['cursor']}", tok)
        resumed = [(l["kind"], l["id"]) for l in _lines(rest) if l.get("type") == "id"]
        ctx.check("an ids-only snapshot resumes after the last line", resumed == tracks[1:], f"{len(resumed)} vs {len(tracks) - 1}")
    fav = next((k for k in id_keys if k[0].startswith("favourite.")), None)
    if fav:
        st, _h, rest = ctx.request("GET", f"/api/v2/sync/snapshot?fields=ids&kinds=favourite,bookmark&after={fav[0]}:{fav[1]}&cursor={ids[0]['cursor']}", tok)
        resumed = {(l["kind"], l["id"]) for l in _lines(rest) if l.get("type") == "id"}
        ctx.check("resuming after a state line sends that state again, whole",
                  {k for k in id_keys if k[0].startswith("favourite.")} <= resumed)


def members(ctx: Ctx, owner: str, owner_id: str, kid: str, kid_id: str) -> None:
    """Owner names in sync: everyone in the household, and those who left."""
    check = ctx.check
    sfx = ctx.sfx
    st, _h, raw = ctx.request("GET", "/api/v2/sync/snapshot?kinds=member", kid)
    people = {l["id"]: l["record"] for l in _lines(raw) if l.get("type") == "record"}
    check("the snapshot names the household's people", owner_id in people and kid_id in people
          and people[owner_id]["display_name"] == f"owner {sfx}" and people[owner_id]["left_at"] is None, str(people.get(owner_id)))
    check("a member record is the user id, names, picture and left_at",
          set(people.get(kid_id, {})) == {"user_id", "display_name", "label", "image", "left_at"}, str(people.get(kid_id)))

    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes?kinds=member", kid)
    cursor = json.loads(raw)["cursor"]
    ctx.call("PATCH", "/api/v2/me", owner, {"display_name": f"Owner Renamed {sfx}"}, expect=(200,))
    page = json.loads(ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "member"}), kid)[2])
    mine = [c for c in page["changes"] if c["id"] == owner_id]
    check("a renamed member is a change of their record", len(mine) == 1 and mine[0]["record"]["display_name"] == f"Owner Renamed {sfx}",
          str(page)[:300])
    cursor = page["cursor"]

    leaver_id, _le, leaver = ctx.make_user("leaver", PW)
    def share_a_song(token: str) -> None:
        ctx.call("POST", "/api/v2/tracks", token, {"upload_id": _upload(ctx, token, "track", "l.mp3"), "filename": "l.mp3",
                                                    "title": f"Leaver Song {sfx}", "artist": f"Leaver Band {sfx}",
                                                    "album": f"Leaver Album {sfx}", "visibility": "family"}, expect=(201,))
    share_a_song(leaver)
    if not ctx.feature("one_family"):
        # Leaving keeps the name: their items left with them, but the household's history names them.
        ctx.call("DELETE", f"/api/v2/family/members/{leaver_id}", owner, expect=(204,))
        page = json.loads(ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "member,track"}), kid)[2])
        left = [c for c in page["changes"] if c["kind"] == "member" and c["id"] == leaver_id]
        check("someone who leaves stays a member record, with left_at and their name",
              bool(left) and left[-1]["op"] == "upsert" and left[-1]["record"]["left_at"] and left[-1]["record"]["display_name"] == f"leaver {sfx}",
              str(left)[:300])
        cursor = page["cursor"]
        leaver_id, _le, leaver = ctx.make_user("leaver2", PW)
        share_a_song(leaver)
    else:
        ctx.skip("someone who leaves stays a member record with their name", "the server has one household: nobody leaves it but by deleting the account")
    ctx.delete_user(leaver_id)
    page = json.loads(ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": cursor, "kinds": "member"}), kid)[2])
    gone = [c for c in page["changes"] if c["id"] == leaver_id]
    check("a deleted account stays a member record: left_at set, no name kept",
          bool(gone) and gone[-1]["op"] == "upsert" and gone[-1]["record"]["left_at"] and gone[-1]["record"]["display_name"] is None,
          str(gone)[:300])
    st, _h, raw = ctx.request("GET", "/api/v2/sync/snapshot?kinds=member", kid)
    people = {l["id"]: l["record"] for l in _lines(raw) if l.get("type") == "record"}
    check("…and is still in the snapshot", leaver_id in people and people[leaver_id]["left_at"], str(people.get(leaver_id)))
    other = ctx.request("GET", "/api/v2/sync/snapshot?kinds=member&fields=ids", kid)
    check("…and in the ids-only snapshot", any(l.get("id") == leaver_id for l in _lines(other[2])))
