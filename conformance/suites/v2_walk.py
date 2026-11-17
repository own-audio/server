# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 walked from its document: the permission matrix, another member's and another household's ids, problem shapes, paging without loss.

Every check here reads `docs/api/openapi-v2.json`, so an operation is covered the day it is in
the document (docs/api-v2-completion-plan.md §4.2):

- **Permissions.** Anonymous callers get 401 on everything not `public`; a member gets 403 or 404
  on `family_admin` and `instance_admin`; a household admin on `instance_admin`; a member who may
  not upload on `can_upload`.
- **Someone else's ids.** Another member of the household tries every operation on the owner's
  private track, book, podcast, episode, playlist, smart playlist and bookmark, and every `owner`
  operation on what the owner shares with the household; the admin of another household (on a
  server with several) tries all of them. None may answer 2xx.
- **Problem shapes.** Every refusal met on the way is `application/problem+json` with a status, a
  title, a `urn:own-audio:problem:` type and a `code`.
- **Paging.** Every list with a `cursor`, in each order it offers, walked two at a time against
  one long page: the same items, none twice, in the same order.
"""
from __future__ import annotations

import json
import pathlib
import re
import urllib.parse
import uuid

from core import Ctx, Skip

NAME = "v2_walk"
PW = "V2WalkTest12345!"

OPENAPI = pathlib.Path(__file__).resolve().parents[2] / "docs" / "api" / "openapi-v2.json"
PROBLEM = "application/problem+json"

# The ids an operation's path is filled with, by the path's first segments.
ID_KINDS = [
    ("/api/v2/tracks/{id}", "track"),
    ("/api/v2/books/{id}", "book"),
    ("/api/v2/podcasts/{id}", "podcast"),
    ("/api/v2/episodes/{id}", "episode"),
    ("/api/v2/playlists/{id}", "playlist"),
    ("/api/v2/smart-playlists/{id}", "smart"),
    ("/api/v2/series/{id}", "series"),
    ("/api/v2/collections/{id}", "collection"),
    ("/api/v2/me/bookmarks/{id}", "bookmark"),
    ("/api/v2/me/favourites/{kind}/{id}", "track"),
    ("/api/v2/me/progress/{kind}/{id}", "track"),
    ("/api/v2/me/ratings/{kind}/{id}", "track"),
    ("/api/v2/me/feedback/{kind}/{id}", "track"),
    ("/api/v2/me/push-tokens/{id}", "push_token"),
    ("/api/v2/library/quarantine/{track_id}", "track"),
    ("/api/v2/family/members/{user_id}", "member"),
    ("/api/v2/files/{id}", "file"),
]

# A body that would succeed on the caller's own item, so a refusal is about the id, not the body.
BODIES = {
    ("PUT", "/visibility"): {"visibility": "family"},
    ("PUT", "/api/v2/me/ratings/{kind}/{id}"): {"rating": 3},
    ("PUT", "/api/v2/me/progress/{kind}/{id}"): {"position_ms": 1000},
    ("PATCH", "/api/v2/playlists/{id}"): {"name": "taken"},
    ("PATCH", "/api/v2/smart-playlists/{id}"): {"name": "taken"},
    ("PATCH", "/api/v2/me/bookmarks/{id}"): {"label": "taken"},
    ("PATCH", "/api/v2/books/{id}"): {"title": "taken"},
    ("PATCH", "/api/v2/series/{id}"): {"name": "taken"},
    ("PATCH", "/api/v2/collections/{id}"): {"name": "taken"},
    ("PUT", "/api/v2/books/{id}/tags"): {"tags": ["taken"]},
    ("PUT", "/api/v2/series/{id}/books"): {"books": []},
    ("POST", "/api/v2/collections/{id}/books"): {"book_ids": []},
    ("PUT", "/api/v2/books/{id}/files/order"): {"file_ids": []},
    ("POST", "/api/v2/books/{id}/identity/search"): {"title": "taken"},
    # Creates, well formed, so a refusal of a member who may not upload is about the right.
    ("POST", "/api/v2/uploads"): {"kind": "track", "filename": "x.mp3", "content_type": "audio/mpeg", "size_bytes": 10},
    ("POST", "/api/v2/tracks"): {"upload_id": str(uuid.uuid4()), "filename": "x.mp3"},
    ("POST", "/api/v2/books"): {"title": "x", "files": []},
    ("POST", "/api/v2/podcasts"): {"title": "x"},
    ("POST", "/api/v2/files"): {"upload_id": "x", "path": "Music/walk/x.jpg"},
    ("POST", "/api/v2/podcasts/{id}/episodes"): {"upload_id": str(uuid.uuid4()), "guid": "g", "title": "t"},
    ("POST", "/api/v2/series"): {"name": "x"},
    ("POST", "/api/v2/contributors"): {"name": "x"},
    ("PUT", "/api/v2/contributors/{id}/image"): {"upload_id": "x"},
    ("PUT", "/api/v2/tracks/{id}/image"): {"upload_id": "x"},
    ("PUT", "/api/v2/playlists/{id}/image"): {"upload_id": "x"},
    ("PUT", "/api/v2/tracks/{id}/lyrics"): {"text": "taken"},
    ("PUT", "/api/v2/me/feedback/{kind}/{id}"): {"value": "dislike"},
    ("POST", "/api/v2/smart-playlists/intent"): {"text": "jazz"},
    ("PUT", "/api/v2/me/queue"): {"items": []},
    ("POST", "/api/v2/me/notifications/ack"): {"ids": []},
    ("POST", "/api/v2/me/push-tokens"): {"platform": "fcm", "token": "walk-token"},
}

# Lists not walked here: the change feed and snapshot have their own checks (suite v2, `sync`),
# an identify run's proposals need a run, and a directory category's shows come from the
# metadata service's catalogue, not from this server (suite v2_podcasts).
NOT_PAGED = {"/api/v2/sync/changes", "/api/v2/sync/snapshot", "/api/v2/identify/runs/{id}/proposals", "/api/v2/podcasts/directory"}


def _ops(doc: dict) -> list[tuple[str, str, dict]]:
    return [(m.upper(), p, o) for p, methods in doc["paths"].items() for m, o in methods.items()
            if m in ("get", "post", "put", "patch", "delete")]


def _random_fill(path: str) -> str:
    return "/".join(str(uuid.uuid4()) if s.startswith("{") else s for s in path.split("/"))


def _body(method: str, path: str):
    if method not in ("POST", "PUT", "PATCH"):
        return None
    for (m, suffix), body in BODIES.items():
        if m == method and (path == suffix or path.endswith(suffix)):
            return body
    return {}


class Walker:
    """Calls that check every refusal's shape on the way."""

    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx
        self.misshapen: list[str] = []

    def call(self, method: str, path: str, token: str | None, body=None) -> int:
        status, headers, raw = self.ctx.request(method, path, token, body)
        if status >= 400 and not self.is_problem(status, headers, raw):
            self.misshapen.append(f"{method} {path.split('?')[0]} → {status} {headers.get('content-type', '')}")
        return status

    @staticmethod
    def is_problem(status: int, headers: dict, raw: bytes) -> bool:
        if not headers.get("content-type", "").startswith(PROBLEM):
            return False
        try:
            doc = json.loads(raw)
        except ValueError:
            return False
        return (doc.get("status") == status and str(doc.get("type", "")).startswith("urn:own-audio:problem:")
                and bool(doc.get("title")) and bool(doc.get("code")))


def run(ctx: Ctx) -> None:
    if not OPENAPI.exists():
        raise Skip(f"{OPENAPI} is not here (run from the server repository)")
    doc = json.loads(OPENAPI.read_text())
    ops = _ops(doc)
    w = Walker(ctx)

    permissions(ctx, w, ops)
    if ctx.feature("one_family"):
        owner_id, _e, owner = ctx.make_user("owner", PW)
        _oid, _oe, other = ctx.make_user("other", PW)
        ids = fixtures(ctx, owner, owner_id)
        someone_elses(ctx, w, ops, ids, other, "another member", "private")
        shared = fixtures(ctx, owner, owner_id, visibility="family")
        owner_only(ctx, w, ops, shared, other)
    else:
        a_id, _e, a_tok = ctx.make_family_admin("a-owner", PW)
        _b, _be, b_tok = ctx.make_family_admin("b-owner", PW)
        ids = fixtures(ctx, a_tok, a_id)
        someone_elses(ctx, w, ops, ids, b_tok, "another household's admin", "private")
        shared = fixtures(ctx, a_tok, a_id, visibility="family")
        someone_elses(ctx, w, ops, shared, b_tok, "another household's admin", "family-shared")
    paging(ctx, w, ops)
    ctx.check("every refusal on the way is problem details with a code", not w.misshapen, "; ".join(w.misshapen[:10]))


def permissions(ctx: Ctx, w: Walker, ops) -> None:
    wrong = []
    for method, path, op in ops:
        perm = op.get("x-permission")
        status = w.call(method, _random_fill(path), None, _body(method, path))
        if perm != "public" and status != 401:
            wrong.append(f"anonymous {method} {path} → {status}")
        if perm == "public" and status == 401:
            wrong.append(f"anonymous {method} {path} (public) → 401")
    ctx.check(f"anonymous: 401 on every operation not public, never on a public one ({len(ops)} checked)", not wrong, "; ".join(wrong[:8]))

    _mid, _me, member = ctx.make_user("member", PW)
    _aid, _ae, admin = ctx.make_family_admin("admin", PW)
    wrong = []
    for method, path, op in ops:
        perm = op.get("x-permission")
        if perm in ("family_admin", "instance_admin"):
            status = w.call(method, _random_fill(path), member, _body(method, path))
            if status not in (403, 404):
                wrong.append(f"member {method} {path} → {status}")
        if perm == "instance_admin":
            status = w.call(method, _random_fill(path), admin, _body(method, path))
            if status not in (403, 404):
                wrong.append(f"household admin {method} {path} → {status}")
    n = sum(o.get("x-permission") in ("family_admin", "instance_admin") for _m, _p, o in ops)
    ctx.check(f"a member gets no admin operation, a household admin no instance-admin one ({n} checked)", not wrong, "; ".join(wrong[:8]))

    if not ctx.feature("one_family"):
        ctx.skip("can_upload", "each account has its own household on this server, where it may upload")
        return
    uid, _ue, no_upload = ctx.make_user("noupload", PW)
    ctx.call("PATCH", f"/api/v2/family/members/{uid}", admin, {"can_upload": False}, expect=(200, 204))
    wrong = []
    checked = 0
    for method, path, op in ops:
        if op.get("x-permission") != "can_upload":
            continue
        checked += 1
        status = w.call(method, _random_fill(path), no_upload, _body(method, path))
        if status not in (403, 404):
            wrong.append(f"{method} {path} → {status}")
    ctx.check(f"a member who may not upload gets 403 on every can_upload operation ({checked} checked)", not wrong, "; ".join(wrong[:8]))


def _upload(ctx: Ctx, token: str, kind: str, name: str) -> str:
    audio = ctx.audio()
    made = ctx.call("POST", "/api/v2/uploads", token,
                    {"kind": kind, "filename": name, "content_type": "audio/mpeg", "size_bytes": len(audio)}, expect=(201,))
    ctx.put_bytes(made["url"], audio, made["headers"]["Content-Type"])
    return made["upload_id"]


def fixtures(ctx: Ctx, token: str, user_id: str, visibility: str = "private") -> dict[str, str]:
    """One of each kind a person owns, kept private or shared with the household."""
    sfx = f"{ctx.sfx}-{visibility}"
    track = ctx.upload_track(token, f"Walk Song {sfx}", artist=f"Walk Band {sfx}", album=f"Walk Album {sfx}", visibility=visibility,
                            content=ctx.tone(), content_type="audio/wav")
    book = ctx.call("POST", "/api/v2/books", token, {
        "title": f"Walk Book {sfx}", "author": f"Walk Writer {sfx}", "visibility": visibility,
        "files": [{"upload_id": _upload(ctx, token, "book_file", f"w-{sfx}.mp3"), "relative_path": "01.mp3"}]}, expect=(201,))
    podcast = ctx.call("POST", "/api/v2/podcasts", token, {"title": f"Walk Show {sfx}", "visibility": visibility}, expect=(200, 201))
    episode = ctx.call("POST", f"/api/v2/podcasts/{podcast['id']}/episodes", token, {
        "upload_id": _upload(ctx, token, "episode", f"e-{sfx}.mp3"), "guid": f"walk-{sfx}", "title": f"Walk Episode {sfx}"},
        expect=(200, 201))
    playlist = ctx.call("POST", "/api/v2/playlists", token, {"name": f"Walk List {sfx}", "track_ids": [track["id"]],
                                                             "visibility": visibility}, expect=(201,))
    rule = {"filters": {"genres": {"include": [f"walk{ctx.sfx}"]}}, "limit": {"kind": "count", "n": 10},
            "sequence": {"max_per_artist": 10, "no_consecutive_artist": False}}
    smart = ctx.call("POST", "/api/v2/smart-playlists", token, {"name": f"Walk Smart {sfx}", "rule": rule}, expect=(201,))
    if visibility != "private":
        ctx.call("PATCH", f"/api/v2/smart-playlists/{smart['id']}", token, {"visibility": visibility}, expect=(200, 204, 400))
    bookmark = ctx.call("POST", "/api/v2/me/bookmarks", token, {"kind": "track", "item_id": track["id"], "position_ms": 1000},
                        expect=(201,))
    series = ctx.call("POST", "/api/v2/series", token, {"name": f"Walk Saga {sfx}", "visibility": visibility,
                                                        "books": [{"book_id": book["id"]}]}, expect=(201,))
    collection = ctx.call("POST", "/api/v2/collections", token, {"name": f"Walk Shelf {sfx}", "visibility": visibility,
                                                                 "book_ids": [book["id"]]}, expect=(201,))
    push = ctx.call("POST", "/api/v2/me/push-tokens", token, {"platform": "apns", "token": f"walk-push-{sfx}"}, expect=(201,))
    picture = ctx.call("POST", "/api/v2/uploads", token, {"kind": "companion", "filename": "walk.jpg", "content_type": "image/jpeg", "size_bytes": 4},
                       expect=(201,))
    ctx.put_bytes(picture["url"], b"\xff\xd8wk", picture["headers"]["Content-Type"])
    file = ctx.call("POST", "/api/v2/files", token, {"upload_id": picture["upload_id"], "attach_to": {"kind": "track", "id": track["id"]},
                                                     "relative_path": f"walk-{sfx}.jpg", "visibility": visibility}, expect=(201,))
    return {"track": track["id"], "book": book["id"], "podcast": podcast["id"], "episode": episode["id"],
            "playlist": playlist["id"], "smart": smart["id"], "bookmark": bookmark["id"], "member": user_id,
            "series": series["id"], "collection": collection["id"], "push_token": push["id"],
            "file": file["id"]}


def _filled(path: str, ids: dict[str, str]) -> str | None:
    for prefix, kind in ID_KINDS:
        if path.startswith(prefix):
            out = path.replace("{kind}", "track") if "{kind}" in path else path
            for name in ("{id}", "{track_id}", "{user_id}"):
                out = out.replace(name, ids[kind], 1) if name in prefix else out
            return None if "{" in out else out
    return None


def someone_elses(ctx: Ctx, w: Walker, ops, ids: dict, token: str, who: str, what: str) -> None:
    reached, checked = [], 0
    for method, path, _op in ops:
        filled = _filled(path, ids)
        if filled is None:
            continue
        checked += 1
        status = w.call(method, filled, token, _body(method, path))
        # Taking back one's own favourite or progress is idempotent, whatever the id: it touches
        # nothing of the owner's and tells nothing about the item.
        if method == "DELETE" and path.startswith("/api/v2/me/") and not path.startswith("/api/v2/me/push-tokens/") and status == 204:
            continue
        if 200 <= status < 300:
            reached.append(f"{method} {path} → {status}")
    ctx.check(f"{who}: no operation on the owner's {what} items answers 2xx ({checked} checked)", not reached, "; ".join(reached[:10]))


def owner_only(ctx: Ctx, w: Walker, ops, ids: dict, token: str) -> None:
    reached, checked = [], 0
    for method, path, op in ops:
        if op.get("x-permission") != "owner":
            continue
        filled = _filled(path, ids)
        if filled is None:
            continue
        checked += 1
        status = w.call(method, filled, token, _body(method, path))
        if 200 <= status < 300:
            reached.append(f"{method} {path} → {status}")
    ctx.check(f"another member: no owner operation on what the owner shares answers 2xx ({checked} checked)",
              not reached, "; ".join(reached[:10]))


def paging(ctx: Ctx, w: Walker, ops) -> None:
    uid, _e, tok = ctx.make_family_admin("pager", PW)
    sfx = ctx.sfx
    tracks = []
    for i in range(5):
        # Two of each title, so an order by title has ties to break.
        tracks.append(ctx.upload_track(tok, f"Page Song {sfx} {i // 2}", artist=f"Page Band {sfx} {i % 3}",
                                       album=f"Page Album {sfx} {i % 2}", genre=f"pagegenre{sfx}{i % 3}",
                                       content=ctx.tone(), content_type="audio/wav"))
    tids = [t["id"] for t in tracks]
    for i in range(3):
        ctx.call("POST", "/api/v2/books", tok, {"title": f"Page Book {sfx} {i // 2}", "author": f"Page Writer {sfx} {i}",
                                                "narrator": f"Page Reader {sfx} {i}",
                                                "files": [{"upload_id": _upload(ctx, tok, "book_file", f"p{i}.mp3"), "relative_path": "01.mp3"}]},
                 expect=(201,))
    shows = [ctx.call("POST", "/api/v2/podcasts", tok, {"title": f"Page Show {sfx} {i // 2}", "author": f"Page Host {sfx} {i}",
                                                         "guid": f"page-show-{sfx}-{i}"}, expect=(200, 201)) for i in range(3)]
    show = shows[0]
    for i in range(3):
        ctx.call("POST", f"/api/v2/podcasts/{show['id']}/episodes", tok, {
            "upload_id": _upload(ctx, tok, "episode", f"pe{i}.mp3"), "guid": f"page-{sfx}-{i}", "title": f"Page Episode {i}"},
            expect=(200, 201))
    page_books = ctx.call("GET", f"/api/v2/books?q={urllib.parse.quote(f'Page Book {sfx}')}", tok)["items"]
    for i, b in enumerate(page_books):
        ctx.call("PUT", f"/api/v2/books/{b['id']}/tags", tok, {"tags": [f"pagetag{sfx}{j}" for j in range(i + 1)]})
    for i in range(3):
        ctx.call("POST", "/api/v2/series", tok, {"name": f"Page Saga {sfx} {i // 2}", "books": [{"book_id": b["id"]} for b in page_books]},
                 expect=(201,))
    shelf = ctx.call("POST", "/api/v2/collections", tok, {"name": f"Page Shelf {sfx}", "book_ids": [b["id"] for b in page_books]},
                     expect=(201,))
    for i in range(3):
        ctx.call("POST", "/api/v2/collections", tok, {"name": f"Page Shelf {sfx} {i}"}, expect=(201,))
    playlist = ctx.call("POST", "/api/v2/playlists", tok, {"name": f"Page List {sfx}", "track_ids": tids + tids[:2]}, expect=(201,))
    for i in range(3):
        ctx.call("POST", "/api/v2/playlists", tok, {"name": f"Page List {sfx} {i}"}, expect=(201,))
    for tid in tids[:4]:
        ctx.call("PUT", f"/api/v2/me/favourites/track/{tid}", tok, expect=(200, 204))
        ctx.call("PUT", f"/api/v2/me/ratings/track/{tid}", tok, {"rating": 4}, expect=(200, 204))
        ctx.call("PUT", f"/api/v2/me/progress/track/{tid}", tok, {"position_ms": 1000}, expect=(200, 204))
        ctx.call("PUT", f"/api/v2/me/feedback/track/{tid}", tok, {"value": "dislike"}, expect=(204,))
    for i in range(3):
        ctx.call("POST", "/api/v2/me/bookmarks", tok, {"kind": "track", "item_id": tids[0], "position_ms": 1000 * (i + 1)}, expect=(201,))
    fills = {"/api/v2/playlists/{id}/entries": playlist["id"], "/api/v2/podcasts/{id}/episodes": show["id"],
             "/api/v2/collections/{id}/books": shelf["id"]}
    kinds = {"/api/v2/me/bookmarks": "track", "/api/v2/me/favourites": "track", "/api/v2/me/progress": "track",
             "/api/v2/me/ratings": "track"}

    def walk(path: str, limit: int):
        items, cursor, pages = [], None, 0
        while True:
            q = f"limit={limit}" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else "")
            url = f"{path}{'&' if '?' in path else '?'}{q}"
            status, _h, raw = ctx.request("GET", url, tok)
            if status != 200:
                w.call("GET", url, tok)  # once more, for the shape check
                return None, status
            page = json.loads(raw)
            items += page.get("items", [])
            cursor, pages = page.get("next"), pages + 1
            if not cursor or pages > 400:
                return items, pages

    def key(item) -> str:
        return str(item.get("id")) if isinstance(item, dict) and item.get("id") else json.dumps(item, sort_keys=True)

    walked = 0
    for method, path, op in ops:
        params = {p["name"]: p for p in op.get("parameters", [])}
        if method != "GET" or "cursor" not in params or path in NOT_PAGED:
            continue
        base = path.replace("{id}", fills[path]) if path in fills else path
        if "{" in base:
            ctx.skip(f"paging {path}", "no id to fill it with")
            continue
        if path in kinds:
            base += f"?kind={kinds[path]}"
        # The orders the description names; words that are other parameters are not orders.
        sorts = [w for w in re.findall(r"`(\w+)`", params["sort"].get("description", "")) if w not in params] \
            if "sort" in params else []
        for sort in [None] + sorts:
            url = base if sort is None else f"{base}{'&' if '?' in base else '?'}sort={sort}"
            label = f"paging {path}" + (f" sort={sort}" if sort else "")
            full, status = walk(url, 500)
            if full is None:
                ctx.fail(label, f"answered {status}")
                continue
            if len(full) < 3:
                ctx.skip(label, f"{len(full)} items: too few to page")
                continue
            # Two at a time, or as many pages as two hundred would make on a big install.
            small, pages = walk(url, max(2, len(full) // 200))
            keys, small_keys = [key(i) for i in full], [key(i) for i in (small or [])]
            walked += 1
            ctx.check(label, small_keys == keys and len(set(small_keys)) == len(small_keys) and pages > 1,
                      f"{len(small_keys)} items in {pages} pages against {len(keys)}; repeats {len(small_keys) - len(set(small_keys))}")
    ctx.check("lists walked page by page", walked > 0, "none had enough items")
