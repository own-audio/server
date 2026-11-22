# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 books (completion plan phase 5): editing a book, its files, cover, tags and identity; series; collections; contributors made, described, pictured and removed.

Each route at each permission level: the owner, a household admin on a shared book, another
member (403 on what is shared, 404 on what is private), a member who may not add to the library,
and another household on a server with several. Also: a reorder survives a reload and reaches
`/sync/changes`; replacing a series' books is one change row of the series; a renamed series
reaches its books' records; collections page without loss or repeats; the tag filter; the
identity routes' checks (a search and `PUT identity` through the suite's book where the book
source answers); `Idempotency-Key` on the creates; series and collections in the snapshot; every
refusal is problem details.
"""
from __future__ import annotations

import json
import struct
import urllib.parse
import uuid
import zlib

from core import Ctx
from suites.v2_sync import _upload

NAME = "v2_books"
PW = "V2BooksTest123!"
PROBLEM = "application/problem+json"


def _problem(answer, status: int, code: str | None = None) -> bool:
    got, headers, body = answer
    if got != status or not headers.get("content-type", "").startswith(PROBLEM):
        return False
    try:
        doc = json.loads(body)
    except ValueError:
        return False
    return (doc.get("status") == status and str(doc.get("type", "")).startswith("urn:own-audio:problem:")
            and bool(doc.get("title")) and (code is None or doc.get("code") == code))


def _png(shade: int = 0) -> bytes:
    side = 16

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    rows = b"".join(b"\x00" + bytes([shade, 120, 200 - shade]) * side for _ in range(side))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


class Feed:
    """One reader's change feed, every row as it came (not folded per item)."""

    def __init__(self, ctx: Ctx, token: str, kinds: str | None = None) -> None:
        self.ctx, self.token, self.kinds = ctx, token, kinds
        self.cursor = ctx.call("GET", "/api/v2/sync/changes", token)["cursor"]

    def read(self) -> list[dict]:
        rows: list[dict] = []
        for _ in range(100):
            q = {"cursor": self.cursor, "limit": "500"}
            if self.kinds:
                q["kinds"] = self.kinds
            page = self.ctx.call("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode(q), self.token)
            rows += page["changes"]
            self.cursor = page["cursor"]
            if not page["has_more"]:
                break
        return rows


def _last(rows: list[dict], kind: str, item: str) -> dict | None:
    found = [r for r in rows if r["kind"] == kind and r["id"] == item]
    return found[-1] if found else None


class Suite:
    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx
        self.n = 0

    def book(self, tok: str, files: int = 3, visibility: str = "family", title: str | None = None) -> dict:
        self.n += 1
        sfx = self.ctx.sfx
        return self.ctx.call("POST", "/api/v2/books", tok, {
            "title": title or f"Books Book {sfx} {self.n}", "author": f"Books Writer {sfx}", "visibility": visibility,
            "files": [{"upload_id": _upload(self.ctx, tok, "book_file", f"b{self.n}-{i}.mp3"), "relative_path": f"0{i + 1}.mp3"}
                      for i in range(files)]}, expect=(201,))

    def image(self, tok: str, shade: int = 0, content_type: str = "image/png", body: bytes | None = None) -> str:
        content = body if body is not None else _png(shade)
        filename = "cover.png" if content_type.startswith("image/") else "notes.txt"
        made = self.ctx.call("POST", "/api/v2/uploads", tok, {"kind": "book_cover", "filename": filename,
                                                               "content_type": content_type, "size_bytes": len(content)}, expect=(201,))
        self.ctx.put_bytes(made["url"], content, made["headers"]["Content-Type"])
        return made["upload_id"]

    def get_book(self, tok: str, book_id: str) -> dict:
        return self.ctx.call("GET", f"/api/v2/books/{book_id}", tok)

    def walk(self, path: str, tok: str, limit: int) -> tuple[list, int]:
        items, cursor, pages = [], None, 0
        while True:
            q = {"limit": str(limit)}
            if cursor:
                q["cursor"] = cursor
            sep = "&" if "?" in path else "?"
            page = self.ctx.call("GET", f"{path}{sep}{urllib.parse.urlencode(q)}", tok)
            items += page["items"]
            cursor, pages = page.get("next"), pages + 1
            if not cursor or pages > 500:
                return items, pages


def run(ctx: Ctx) -> None:
    s = Suite(ctx)
    one_household = ctx.feature("one_family")
    owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    member_id, member_email, member = ctx.make_user("member", PW)
    if not one_household:
        inv = ctx.call("POST", "/api/v1/family/invites", owner, {"email": member_email, "role": "member"})
        ctx.call("POST", "/api/v1/family/invites/accept", member, {"code": inv["code"]})
    outsider = None
    if not one_household:
        _xid, _xe, outsider = ctx.make_family_admin("outsider", PW)

    details(ctx, s, owner, member, outsider)
    files(ctx, s, owner, member, member_id, outsider)
    identity(ctx, s, owner, member)
    cover(ctx, s, owner, member, member_id)
    tags(ctx, s, owner, member)
    series(ctx, s, owner, member, member_id, outsider)
    collections(ctx, s, owner, member, outsider)
    contributors(ctx, s, owner, member, member_id)
    snapshot(ctx, s, owner)


def _no_upload(ctx: Ctx, owner: str, label: str) -> tuple[str, str]:
    """A household member whose right to add to the library the admin took away."""
    uid, email, tok = ctx.make_user(label, PW)
    if not ctx.feature("one_family"):
        inv = ctx.call("POST", "/api/v1/family/invites", owner, {"email": email, "role": "member"})
        ctx.call("POST", "/api/v1/family/invites/accept", tok, {"code": inv["code"]})
    ctx.call("PATCH", f"/api/v2/family/members/{uid}", owner, {"can_upload": False}, expect=(200, 204))
    return uid, tok


def details(ctx: Ctx, s: Suite, owner: str, member: str, outsider: str | None) -> None:
    check = ctx.check
    b = s.book(owner)
    private = s.book(owner, visibility="private")
    feed = Feed(ctx, member, "book")
    st, _h, raw = ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {
        "title": f"Edited {ctx.sfx}", "description": "A new start.", "publisher": "Public Domain Press",
        "published_year": 1815, "isbn": "9780000000001"})
    got = json.loads(raw) if st == 200 else {}
    check("PATCH /books/{id}: the owner corrects title, description, publisher, year and ISBN",
          st == 200 and got.get("title") == f"Edited {ctx.sfx}" and got.get("description") == "A new start."
          and got.get("publisher") == "Public Domain Press" and got.get("published_year") == 1815 and got.get("isbn") == "9780000000001",
          f"{st} {raw[:300]!r}")
    check("…the answer is the whole book, files and tags included", isinstance(got.get("files"), list) and got.get("tags") == [])
    again = s.get_book(member, b["id"])
    check("…and a reload by another member shows it", again["title"] == f"Edited {ctx.sfx}" and again["published_year"] == 1815)
    row = _last(feed.read(), "book", b["id"])
    check("…and the change feed sends the book with the new title",
          row is not None and row["op"] == "upsert" and row["record"]["title"] == f"Edited {ctx.sfx}")
    st, _h, raw = ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {"description": None, "published_year": None})
    got = json.loads(raw) if st == 200 else {}
    check("PATCH /books/{id}: null clears a field, an absent one stays",
          st == 200 and got.get("description") is None and got.get("published_year") is None and got.get("publisher") == "Public Domain Press")
    check("PATCH /books/{id}: an empty title is 400 invalid_body",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {"title": "  "}), 400, "invalid_body"))
    check("PATCH /books/{id}: a year out of range is 400 invalid_body",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {"published_year": 0}), 400, "invalid_body"))
    check("PATCH /books/{id}: another member on a shared book is 403",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}", member, {"title": "Mine"}), 403, "forbidden"))
    check("…on a private one 404",
          _problem(ctx.request("PATCH", f"/api/v2/books/{private['id']}", member, {"title": "Mine"}), 404, "not_found"))
    check("…an unknown book 404", _problem(ctx.request("PATCH", f"/api/v2/books/{uuid.uuid4()}", owner, {"title": "x"}), 404, "not_found"))
    if outsider:
        check("…another household's admin 404",
              _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}", outsider, {"title": "Theirs"}), 404, "not_found"))
    else:
        ctx.skip("PATCH /books/{id}: another household", "one household on this server")
    check("…and nothing of that changed it", s.get_book(owner, b["id"])["title"] == f"Edited {ctx.sfx}")
    language(ctx, s, owner, member)


def language(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    """A book's language (the owner's to set) and origin (the server's only), revision 19."""
    check = ctx.check
    b = s.book(owner)
    fresh = s.get_book(owner, b["id"])
    check("GET /books/{id}: an uploaded book has language and origin, both null",
          "language" in fresh and fresh["language"] is None and "origin" in fresh and fresh["origin"] is None)
    feed = Feed(ctx, member, "book")
    st, _h, raw = ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {
        "language": "pt-BR", "origin": {"kind": "machine_translated", "translator": "google"}})
    got = json.loads(raw) if st == 200 else {}
    check("PATCH /books/{id}: the owner sets the language", st == 200 and got.get("language") == "pt-BR", f"{st} {raw[:300]!r}")
    check("…but origin is the server's: a client's is ignored", st == 200 and got.get("origin") is None)
    row = _last(feed.read(), "book", b["id"])
    check("…and the change feed sends the book with its language", row is not None and row["record"].get("language") == "pt-BR")
    st, _h, v1 = ctx.request("GET", f"/api/v1/audiobooks/{b['id']}", owner)
    v1_book = json.loads(v1) if st == 200 else {}
    check("GET /api/v1/audiobooks/{id}: language and origin there too (revision 29)",
          v1_book.get("language") == "pt-BR" and "origin" in v1_book and v1_book["origin"] is None, f"{st} {v1[:300]!r}")
    check("PATCH /books/{id}: a language that is not a BCP 47 tag is 400 invalid_body",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {"language": "english"}), 400, "invalid_body"))
    st, _h, raw = ctx.request("PATCH", f"/api/v2/books/{b['id']}", owner, {"language": None})
    check("…null clears it", st == 200 and json.loads(raw).get("language") is None, f"{st} {raw[:200]!r}")


def files(ctx: Ctx, s: Suite, owner: str, member: str, member_id: str, outsider: str | None) -> None:
    check = ctx.check
    b = s.book(owner, files=2)
    key = {"Idempotency-Key": f"v2books-files-{uuid.uuid4()}"}
    body = {"files": [{"upload_id": _upload(ctx, owner, "book_file", "c.mp3"), "relative_path": "03.mp3", "title": "Three"},
                      {"upload_id": _upload(ctx, owner, "book_file", "a.mp3"), "relative_path": "00.mp3"}]}
    first = ctx.request("POST", f"/api/v2/books/{b['id']}/files", owner, body, headers=key)
    got = json.loads(first[2]) if first[0] == 200 else {}
    paths = [f.get("relative_path") for f in got.get("files", [])]
    check("POST /books/{id}/files: many files at once, in play order by their paths",
          first[0] == 200 and paths == ["00.mp3", "01.mp3", "02.mp3", "03.mp3"] and [f["position"] for f in got["files"]] == [1, 2, 3, 4],
          f"{first[0]} {paths}")
    again = ctx.request("POST", f"/api/v2/books/{b['id']}/files", owner, body, headers=key)
    check("…a retry with the same Idempotency-Key answers the first answer",
          again[0] == 200 and again[1].get("idempotent-replayed") == "true" and json.loads(again[2]) == got)
    resend = ctx.call("POST", f"/api/v2/books/{b['id']}/files", owner,
                      {"files": [{"upload_id": _upload(ctx, owner, "book_file", "c2.mp3"), "relative_path": "03.mp3"}]})
    check("…a path the book has already answers its existing file, nothing added", len(resend["files"]) == 4)
    check("POST /books/{id}/files: a file without relative_path is 400 invalid_body",
          _problem(ctx.request("POST", f"/api/v2/books/{b['id']}/files", owner,
                               {"files": [{"upload_id": _upload(ctx, owner, "book_file", "x.mp3")}]}), 400, "invalid_body"))
    check("…no files at all is 400 invalid_body",
          _problem(ctx.request("POST", f"/api/v2/books/{b['id']}/files", owner, {"files": []}), 400, "invalid_body"))
    check("…an upload that is not the household's is 404",
          _problem(ctx.request("POST", f"/api/v2/books/{b['id']}/files", owner,
                               {"files": [{"upload_id": "nope", "relative_path": "09.mp3"}]}), 404, "not_found"))
    check("…another member on a shared book is 403",
          _problem(ctx.request("POST", f"/api/v2/books/{b['id']}/files", member,
                               {"files": [{"upload_id": _upload(ctx, member, "book_file", "m.mp3"), "relative_path": "07.mp3"}]}), 403))
    # A member whose right to add was taken away, on their own book.
    nid, noup = _no_upload(ctx, owner, "files-noup")
    ctx.call("PATCH", f"/api/v2/family/members/{nid}", owner, {"can_upload": True}, expect=(200, 204))
    theirs = s.book(noup, files=1)
    upload = _upload(ctx, noup, "book_file", "n.mp3")
    ctx.call("PATCH", f"/api/v2/family/members/{nid}", owner, {"can_upload": False}, expect=(200, 204))
    check("…the owner who may no longer add to the library is 403",
          _problem(ctx.request("POST", f"/api/v2/books/{theirs['id']}/files", noup,
                               {"files": [{"upload_id": upload, "relative_path": "02.mp3"}]}), 403, "forbidden"))

    # Rename.
    fid = got["files"][1]["id"]
    st, _h, raw = ctx.request("PATCH", f"/api/v2/books/{b['id']}/files/{fid}", owner, {"title": "Chapter One"})
    check("PATCH /books/{id}/files/{file_id}: renames the file", st == 200 and json.loads(raw).get("title") == "Chapter One", f"{st} {raw[:200]!r}")
    check("…an empty title is 400 invalid_body",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}/files/{fid}", owner, {"title": " "}), 400, "invalid_body"))
    check("…an unknown file 404",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}/files/{uuid.uuid4()}", owner, {"title": "x"}), 404, "not_found"))
    check("…another member on a shared book 403",
          _problem(ctx.request("PATCH", f"/api/v2/books/{b['id']}/files/{fid}", member, {"title": "x"}), 403, "forbidden"))
    mine = s.book(member, files=2)
    mfid = mine["files"][0]["id"]
    check("…a household admin on a member's shared book may (v1's rule)",
          ctx.status_of("PATCH", f"/api/v2/books/{mine['id']}/files/{mfid}", owner, {"title": "Tidied"}) == 200
          and s.get_book(member, mine["id"])["files"][0]["title"] == "Tidied")

    # Reorder: survives a reload and reaches the change feed, once.
    feed = Feed(ctx, member, "book")
    ids = [f["id"] for f in got["files"]]
    wanted = [ids[3], ids[0]]
    st = ctx.status_of("PUT", f"/api/v2/books/{b['id']}/files/order", owner, {"file_ids": wanted})
    reloaded = [f["id"] for f in s.get_book(member, b["id"])["files"]]
    check("PUT /books/{id}/files/order: named files first, the rest after in their order, after a reload",
          st == 204 and reloaded == [ids[3], ids[0], ids[1], ids[2]], f"{st} {reloaded} {ids}")
    row = _last(feed.read(), "book", b["id"])
    check("…and /sync/changes sends the book with the files in the new order",
          row is not None and [f["id"] for f in row["record"]["files"]] == reloaded
          and [f["position"] for f in row["record"]["files"]] == [1, 2, 3, 4])
    check("…an id twice is 400 invalid_file",
          _problem(ctx.request("PUT", f"/api/v2/books/{b['id']}/files/order", owner, {"file_ids": [ids[0], ids[0]]}), 400, "invalid_file"))
    check("…another book's file is 400 invalid_file",
          _problem(ctx.request("PUT", f"/api/v2/books/{b['id']}/files/order", owner, {"file_ids": [mfid]}), 400, "invalid_file"))
    check("…another member 403",
          _problem(ctx.request("PUT", f"/api/v2/books/{b['id']}/files/order", member, {"file_ids": []}), 403, "forbidden"))
    if outsider:
        check("…another household 404",
              _problem(ctx.request("PUT", f"/api/v2/books/{b['id']}/files/order", outsider, {"file_ids": []}), 404, "not_found"))

    # Delete one file; never the last.
    gone = reloaded[-1]
    st = ctx.status_of("DELETE", f"/api/v2/books/{b['id']}/files/{gone}", owner)
    left = [f["id"] for f in s.get_book(owner, b["id"])["files"]]
    check("DELETE /books/{id}/files/{file_id}: one file goes, the others stay", st == 204 and gone not in left and len(left) == 3)
    check("…again 404", _problem(ctx.request("DELETE", f"/api/v2/books/{b['id']}/files/{gone}", owner), 404, "not_found"))
    check("…another member 403", _problem(ctx.request("DELETE", f"/api/v2/books/{b['id']}/files/{left[0]}", member), 403, "forbidden"))
    single = s.book(owner, files=1)
    check("…a book's last file is 409 last_file",
          _problem(ctx.request("DELETE", f"/api/v2/books/{single['id']}/files/{single['files'][0]['id']}", owner), 409, "last_file"))
    _ = member_id


def identity(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    check = ctx.check
    b = s.book(owner, title="Pride and Prejudice")
    private = s.book(owner, visibility="private")
    path = f"/api/v2/books/{b['id']}/identity"
    check("POST /books/{id}/identity/search: neither title nor author is 400 invalid_body",
          _problem(ctx.request("POST", f"{path}/search", owner, {"title": " "}), 400, "invalid_body"))
    check("…a limit over 40 is 400 invalid_limit",
          _problem(ctx.request("POST", f"{path}/search", owner, {"title": "Emma", "limit": 41}), 400, "invalid_limit"))
    check("…another member on a shared book 403",
          _problem(ctx.request("POST", f"{path}/search", member, {"title": "Emma"}), 403, "forbidden"))
    check("…on a private one 404",
          _problem(ctx.request("POST", f"/api/v2/books/{private['id']}/identity/search", member, {"title": "Emma"}), 404, "not_found"))
    check("PUT /books/{id}/identity: an id no search gives out is 400 invalid_volume_id, before any request",
          _problem(ctx.request("PUT", path, owner, {"volume_id": "ol:../x", "fields": {"cover": False}}), 400, "invalid_volume_id"))
    check("…another member 403", _problem(ctx.request("PUT", path, member, {"volume_id": "ol:OL1W"}), 403, "forbidden"))
    st, _h, raw = ctx.request("POST", f"{path}/search", owner, {"title": "Pride and Prejudice", "author": "Jane Austen", "limit": 5})
    if st in (502, 503):
        check("…a book source that does not answer is problem details", _problem((st, _h, raw), st))
        ctx.skip("identity through the suite's book", f"the book source did not answer ({st})")
        return
    found = json.loads(raw).get("items", []) if st == 200 else []
    check("POST /books/{id}/identity/search: matches from the household's source, best first",
          st == 200 and all(m.get("volume_id") and m.get("source") and m.get("title") for m in found)
          and [m["score"] for m in found] == sorted((m["score"] for m in found), reverse=True), f"{st} {raw[:300]!r}")
    if not found:
        ctx.skip("PUT /books/{id}/identity through the suite's book", "the source found nothing")
        return
    match = found[0]
    feed = Feed(ctx, owner, "book")
    st, _h, raw = ctx.request("PUT", path, owner, {"volume_id": match["volume_id"],
                                                   "fields": {"cover": False, "author": False, "title": True, "description": False}})
    if st in (502, 503):
        ctx.skip("PUT /books/{id}/identity through the suite's book", f"the book source did not answer ({st})")
        return
    got = json.loads(raw) if st == 200 else {}
    check("PUT /books/{id}/identity: the match's title on the suite's book, the rest as asked",
          st == 200 and got.get("title") == match["title"], f"{st} {raw[:300]!r}")
    check("…and the change feed sends the book", _last(feed.read(), "book", b["id"]) is not None)


def cover(ctx: Ctx, s: Suite, owner: str, member: str, member_id: str) -> None:
    check = ctx.check
    b = s.book(owner)
    st, _h, raw = ctx.request("PUT", f"/api/v2/books/{b['id']}/image", owner, {"upload_id": s.image(owner, 10)})
    first = (json.loads(raw) if st == 200 else {}).get("image") or ""
    check("PUT /books/{id}/image: the book with a link to its new cover", st == 200 and "/api/v2/images/" in first, f"{st} {raw[:200]!r}")
    if first:
        status = ctx.status_of("GET", first)
        check("…the link serves the picture without a token", status == 200, str(status))
    st, _h, raw = ctx.request("PUT", f"/api/v2/books/{b['id']}/image", owner, {"upload_id": s.image(owner, 90)})
    second = (json.loads(raw) if st == 200 else {}).get("image") or ""
    check("…a new picture is a new link", st == 200 and second and second != first)
    words = ctx.request("PUT", f"/api/v2/books/{b['id']}/image", owner, {"upload_id": s.image(owner, body=b"plain words", content_type="text/plain")})
    check("…an upload that is not an image is 400 not_an_image", _problem(words, 400, "not_an_image"), f"{words[0]} {words[2][:200]!r}")
    check("…another household's or an unknown upload is 404",
          _problem(ctx.request("PUT", f"/api/v2/books/{b['id']}/image", owner, {"upload_id": f"families/{uuid.uuid4()}/x.png"}), 404))
    check("…another member on a shared book 403",
          _problem(ctx.request("PUT", f"/api/v2/books/{b['id']}/image", member, {"upload_id": s.image(member)}), 403, "forbidden"))
    nid, noup = _no_upload(ctx, owner, "cover-noup")
    ctx.call("PATCH", f"/api/v2/family/members/{nid}", owner, {"can_upload": True}, expect=(200, 204))
    theirs, upload = s.book(noup, files=1), s.image(noup)
    ctx.call("PATCH", f"/api/v2/family/members/{nid}", owner, {"can_upload": False}, expect=(200, 204))
    check("…the owner who may not add to the library 403",
          _problem(ctx.request("PUT", f"/api/v2/books/{theirs['id']}/image", noup, {"upload_id": upload}), 403, "forbidden"))
    check("DELETE /books/{id}/image: another member 403", _problem(ctx.request("DELETE", f"/api/v2/books/{b['id']}/image", member), 403, "forbidden"))
    st = ctx.status_of("DELETE", f"/api/v2/books/{b['id']}/image", owner)
    check("DELETE /books/{id}/image: the cover is gone", st == 204 and s.get_book(owner, b["id"]).get("image") is None)
    check("…again, still 204", ctx.status_of("DELETE", f"/api/v2/books/{b['id']}/image", owner) == 204)
    _ = member_id


def tags(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    check = ctx.check
    sfx = ctx.sfx
    tag = f"Classics{sfx}"
    books = [s.book(owner, files=1) for _ in range(3)]
    other = s.book(owner, files=1)
    private = s.book(owner, files=1, visibility="private")
    feed = Feed(ctx, member, "book")
    st, _h, raw = ctx.request("PUT", f"/api/v2/books/{books[0]['id']}/tags", owner, {"tags": [f" {tag} ", f"sci-fi{sfx}", tag.lower()]})
    got = json.loads(raw) if st == 200 else {}
    check("PUT /books/{id}/tags: the book's tags, trimmed, each once whatever its case",
          st == 200 and sorted(t.lower() for t in got.get("tags", [])) == sorted([tag.lower(), f"sci-fi{sfx}"]), f"{st} {raw[:200]!r}")
    check("…and the change feed sends the book with them",
          (row := _last(feed.read(), "book", books[0]["id"])) is not None and len(row["record"].get("tags") or []) == 2)
    for b in books[1:]:
        ctx.call("PUT", f"/api/v2/books/{b['id']}/tags", owner, {"tags": [tag.upper()]})
    ctx.call("PUT", f"/api/v2/books/{private['id']}/tags", owner, {"tags": [tag, f"secret{sfx}"]})
    check("…a tag keeps the spelling it was first given", s.get_book(owner, books[1]["id"])["tags"] == [tag])
    full, _p = s.walk(f"/api/v2/books?tag={urllib.parse.quote(tag.lower())}", member, 500)
    small, pages = s.walk(f"/api/v2/books?tag={urllib.parse.quote(tag.lower())}", member, 1)
    ids = [b["id"] for b in full]
    check("GET /books?tag=: the books with the tag the caller can see, any case",
          sorted(ids) == sorted(b["id"] for b in books) and other["id"] not in ids and private["id"] not in ids, f"{ids}")
    check("…paged one at a time without loss or repeats", [b["id"] for b in small] == ids and pages >= 3)
    seen, _p = s.walk("/api/v2/book-tags", member, 500)
    by_name = {t["name"].lower(): t for t in seen}
    check("GET /book-tags: each tag once with how many books the caller can see",
          by_name.get(tag.lower(), {}).get("book_count") == 3 and f"sci-fi{sfx}" in by_name, f"{by_name.get(tag.lower())}")
    check("…a tag only on someone's private book is not shown", f"secret{sfx}" not in by_name)
    mine, _p = s.walk("/api/v2/book-tags", owner, 500)
    check("…its owner sees it, and the count with the private book", {t["name"].lower(): t["book_count"] for t in mine}.get(tag.lower()) == 4)
    small, pages = s.walk("/api/v2/book-tags", member, 1)
    check("GET /book-tags: paged one at a time without loss or repeats",
          [t["id"] for t in small] == [t["id"] for t in seen] and len({t["id"] for t in small}) == len(small))
    check("PUT /books/{id}/tags: an empty tag is 400 invalid_tag",
          _problem(ctx.request("PUT", f"/api/v2/books/{other['id']}/tags", owner, {"tags": ["ok", " "]}), 400, "invalid_tag"))
    check("…more than 50 is 400 too_many",
          _problem(ctx.request("PUT", f"/api/v2/books/{other['id']}/tags", owner, {"tags": [f"t{i}" for i in range(51)]}), 400, "too_many"))
    check("…another member 403", _problem(ctx.request("PUT", f"/api/v2/books/{books[0]['id']}/tags", member, {"tags": []}), 403, "forbidden"))
    check("…[] takes them all away", ctx.call("PUT", f"/api/v2/books/{books[0]['id']}/tags", owner, {"tags": []})["tags"] == [])


def series(ctx: Ctx, s: Suite, owner: str, member: str, member_id: str, outsider: str | None) -> None:
    check = ctx.check
    sfx = ctx.sfx
    b1, b2, b3 = s.book(owner, files=1), s.book(owner, files=1), s.book(owner, files=1)
    hidden = s.book(owner, files=1, visibility="private")
    key = {"Idempotency-Key": f"v2books-series-{uuid.uuid4()}"}
    body = {"name": f"Saga {sfx}", "description": "Three books.", "visibility": "family",
            "books": [{"book_id": b1["id"]}, {"book_id": b2["id"]}, {"book_id": hidden["id"], "position": 9}]}
    first = ctx.request("POST", "/api/v2/series", owner, body, headers=key)
    made = json.loads(first[2]) if first[0] == 201 else {}
    check("POST /series: a series with its books, numbered where no place is given",
          first[0] == 201 and [(x["book_id"], x["position"]) for x in made.get("books", [])] == [(b1["id"], 1), (b2["id"], 2), (hidden["id"], 9)]
          and made.get("visibility") == "family", f"{first[0]} {first[2][:300]!r}")
    again = ctx.request("POST", "/api/v2/series", owner, body, headers=key)
    check("…a retry with the same Idempotency-Key answers the same series", again[0] == 201 and json.loads(again[2])["id"] == made.get("id"))
    sid = made["id"]
    seen = ctx.call("GET", f"/api/v2/series/{sid}", member)
    check("GET /series/{id}: another member sees the shared series with only the books they can see",
          [x["book_id"] for x in seen["books"]] == [b1["id"], b2["id"]], f"{seen['books']}")
    check("…and the book's record names the series and its place",
          s.get_book(member, b2["id"]).get("series") == [{"series_id": sid, "name": f"Saga {sfx}", "position": 2.0}])
    _nid, noup = _no_upload(ctx, owner, "series-noup")
    check("POST /series: a member who may not add to the library 403",
          _problem(ctx.request("POST", "/api/v2/series", noup, {"name": "No"}), 403, "forbidden"))
    check("…an empty name 400 invalid_body", _problem(ctx.request("POST", "/api/v2/series", owner, {"name": " "}), 400, "invalid_body"))
    check("…a book nobody can see 400 invalid_book",
          _problem(ctx.request("POST", "/api/v2/series", owner, {"name": "X", "books": [{"book_id": str(uuid.uuid4())}]}), 400, "invalid_book"))
    check("…a book twice 400 invalid_book",
          _problem(ctx.request("POST", "/api/v2/series", owner, {"name": "X", "books": [{"book_id": b1["id"]}, {"book_id": b1["id"]}]}), 400, "invalid_book"))

    # The whole list in one call: one change row of the series; the books follow.
    series_feed, book_feed = Feed(ctx, member, "series"), Feed(ctx, member, "book")
    st, _h, raw = ctx.request("PUT", f"/api/v2/series/{sid}/books", owner,
                              {"books": [{"book_id": b3["id"], "position": 1}, {"book_id": b1["id"], "position": 1.5}, {"book_id": b2["id"], "position": 2}]})
    got = json.loads(raw) if st == 200 else {}
    check("PUT /series/{id}/books: the list in its new order and places; a book of its own left out leaves",
          st == 200 and [(x["book_id"], x["position"]) for x in got.get("books", [])] == [(b3["id"], 1), (b1["id"], 1.5), (b2["id"], 2)],
          f"{st} {raw[:300]!r}")
    rows = [r for r in series_feed.read() if r["id"] == sid]
    check("…arrives in /sync/changes as one change row of the series, its record as the reader sees it",
          len(rows) == 1 and rows[0]["op"] == "upsert" and [x["book_id"] for x in rows[0]["record"]["books"]] == [b3["id"], b1["id"], b2["id"]],
          f"{len(rows)} rows")
    brows = book_feed.read()
    check("…and each book whose place changed comes with its new place",
          (r := _last(brows, "book", b1["id"])) is not None and r["record"]["series"][0]["position"] == 1.5
          and _last(brows, "book", b3["id"]) is not None)
    check("…a reload shows the same", [x["book_id"] for x in ctx.call("GET", f"/api/v2/series/{sid}", member)["books"]] == [b3["id"], b1["id"], b2["id"]])
    # A member's book in the series, then made private: its owner no longer sees it, and a
    # replace by the series' owner leaves it where it is.
    theirs = s.book(member, files=1)
    ctx.call("PUT", f"/api/v2/series/{sid}/books", owner, {"books": [{"book_id": b1["id"]}, {"book_id": theirs["id"]}]})
    ctx.call("PUT", f"/api/v2/books/{theirs['id']}/visibility", member, {"visibility": "private"})
    st, _h, raw = ctx.request("PUT", f"/api/v2/series/{sid}/books", owner, {"books": [{"book_id": b1["id"]}]})
    check("…books left out leave the series",
          st == 200 and [x["book_id"] for x in json.loads(raw)["books"]] == [b1["id"]], f"{st} {raw[:200]!r}")
    ctx.call("PUT", f"/api/v2/books/{theirs['id']}/visibility", member, {"visibility": "family"})
    check("…and one the owner could not see stayed",
          [x["book_id"] for x in ctx.call("GET", f"/api/v2/series/{sid}", owner)["books"]] == [b1["id"], theirs["id"]])
    ctx.call("PUT", f"/api/v2/series/{sid}/books", owner, {"books": [{"book_id": b1["id"]}]})
    check("…another member 403", _problem(ctx.request("PUT", f"/api/v2/series/{sid}/books", member, {"books": []}), 403, "forbidden"))

    # Rename: the series and the books that show its name.
    series_feed, book_feed = Feed(ctx, member, "series"), Feed(ctx, member, "book")
    st, _h, raw = ctx.request("PATCH", f"/api/v2/series/{sid}", owner, {"name": f"Saga {sfx} Renamed", "description": None})
    check("PATCH /series/{id}: renamed, description cleared",
          st == 200 and json.loads(raw)["name"] == f"Saga {sfx} Renamed" and json.loads(raw)["description"] is None)
    check("…the series' change row and its book's record with the new name",
          _last(series_feed.read(), "series", sid) is not None
          and (r := _last(book_feed.read(), "book", b1["id"])) is not None and r["record"]["series"][0]["name"] == f"Saga {sfx} Renamed")
    check("…another member 403", _problem(ctx.request("PATCH", f"/api/v2/series/{sid}", member, {"name": "Mine"}), 403, "forbidden"))
    if outsider:
        check("…another household 404", _problem(ctx.request("GET", f"/api/v2/series/{sid}", outsider), 404, "not_found"))
    # Unshared: gone for the member, as a removal.
    series_feed = Feed(ctx, member, "series")
    ctx.call("PATCH", f"/api/v2/series/{sid}", owner, {"visibility": "private"})
    check("…made private: another member's GET is 404 and the feed sends a removal",
          _problem(ctx.request("GET", f"/api/v2/series/{sid}", member), 404, "not_found")
          and (r := _last(series_feed.read(), "series", sid)) is not None and r["op"] == "delete")
    check("…and the book no longer names it for them", s.get_book(member, b1["id"]).get("series") == [])
    ctx.call("PATCH", f"/api/v2/series/{sid}", owner, {"visibility": "family"})

    # Paging the list.
    for i in range(3):
        ctx.call("POST", "/api/v2/series", owner, {"name": f"Saga {sfx} {i}", "visibility": "family"}, expect=(201,))
    full, _p = s.walk("/api/v2/series", member, 500)
    small, pages = s.walk("/api/v2/series", member, 2)
    check("GET /series: paged two at a time without loss or repeats",
          [x["id"] for x in small] == [x["id"] for x in full] and len({x["id"] for x in small}) == len(small) and pages > 1
          and sum(x["name"].startswith(f"Saga {sfx}") for x in full) >= 4)

    # Delete: a member 403, a household admin on a member's shared series may, the owner may.
    check("DELETE /series/{id}: another member 403", _problem(ctx.request("DELETE", f"/api/v2/series/{sid}", member), 403, "forbidden"))
    series_feed = Feed(ctx, member, "series")
    check("…the owner 204", ctx.status_of("DELETE", f"/api/v2/series/{sid}", owner) == 204)
    check("…a removal in the feed, the books stay and name it no more",
          (r := _last(series_feed.read(), "series", sid)) is not None and r["op"] == "delete"
          and s.get_book(member, b1["id"]).get("series") == [])
    theirs = ctx.call("POST", "/api/v2/series", member, {"name": f"Member Saga {sfx}", "visibility": "family"}, expect=(201,))
    check("…a household admin on a member's shared series 204",
          ctx.status_of("DELETE", f"/api/v2/series/{theirs['id']}", owner) == 204)
    _ = member_id


def collections(ctx: Ctx, s: Suite, owner: str, member: str, outsider: str | None) -> None:
    check = ctx.check
    sfx = ctx.sfx
    books = [s.book(owner, files=1) for _ in range(5)]
    hidden = s.book(owner, files=1, visibility="private")
    key = {"Idempotency-Key": f"v2books-coll-{uuid.uuid4()}"}
    body = {"name": f"Shelf {sfx}", "visibility": "family", "book_ids": [books[0]["id"], str(uuid.uuid4())]}
    first = ctx.request("POST", "/api/v2/collections", member, body, headers=key)
    made = json.loads(first[2]) if first[0] == 201 else {}
    check("POST /collections: any member makes one; a book they cannot see is left out",
          first[0] == 201 and made.get("book_count") == 1 and made.get("visibility") == "family", f"{first[0]} {first[2][:300]!r}")
    again = ctx.request("POST", "/api/v2/collections", member, body, headers=key)
    check("…a retry with the same Idempotency-Key answers the same collection", again[0] == 201 and json.loads(again[2])["id"] == made.get("id"))
    cid = made["id"]
    feed = Feed(ctx, owner, "collection")
    st, _h, raw = ctx.request("POST", f"/api/v2/collections/{cid}/books", member,
                              {"book_ids": [b["id"] for b in books[1:]] + [books[0]["id"], hidden["id"]]})
    check("POST /collections/{id}/books: added in order, one already in it and one unseen left out",
          st == 200 and json.loads(raw)["book_count"] == 5, f"{st} {raw[:200]!r}")
    rows = [r for r in feed.read() if r["id"] == cid]
    check("…one change row of the collection, with its count", len(rows) == 1 and rows[0]["record"]["book_count"] == 5, f"{rows}")
    full, _p = s.walk(f"/api/v2/collections/{cid}/books", owner, 500)
    small, pages = s.walk(f"/api/v2/collections/{cid}/books", owner, 2)
    check("GET /collections/{id}/books: in the order added, paged two at a time without loss or repeats",
          [e["book"]["id"] for e in full] == [b["id"] for b in books]
          and [e["book"]["id"] for e in small] == [e["book"]["id"] for e in full] and pages == 3, f"{pages} pages")
    check("…each entry a book with its place", all(isinstance(e["position"], int) and e["book"].get("title") for e in full))
    check("POST /collections/{id}/books: another member 403",
          _problem(ctx.request("POST", f"/api/v2/collections/{cid}/books", owner, {"book_ids": [hidden["id"]]}), 403, "forbidden"))
    check("…more than 1000 is 400 too_many",
          _problem(ctx.request("POST", f"/api/v2/collections/{cid}/books", member, {"book_ids": [str(uuid.uuid4())] * 1001}), 400, "too_many"))
    check("DELETE /collections/{id}/books/{book_id}: removed", ctx.status_of("DELETE", f"/api/v2/collections/{cid}/books/{books[2]['id']}", member) == 204
          and ctx.call("GET", f"/api/v2/collections/{cid}", owner)["book_count"] == 4)
    check("…again 404", _problem(ctx.request("DELETE", f"/api/v2/collections/{cid}/books/{books[2]['id']}", member), 404, "not_found"))
    check("…another member 403", _problem(ctx.request("DELETE", f"/api/v2/collections/{cid}/books/{books[0]['id']}", owner), 403, "forbidden"))
    # The owner's book trashed: the collection counts what is left, and says so in the feed.
    feed = Feed(ctx, member, "collection")
    ctx.call("DELETE", f"/api/v2/books/{books[3]['id']}", owner)
    check("A book in the trash leaves the collection's count, and the feed sends the collection again",
          ctx.call("GET", f"/api/v2/collections/{cid}", member)["book_count"] == 3
          and (r := _last(feed.read(), "collection", cid)) is not None and r["record"]["book_count"] == 3)
    st, _h, raw = ctx.request("PATCH", f"/api/v2/collections/{cid}", member, {"name": f"Shelf {sfx} B", "description": "Mine."})
    check("PATCH /collections/{id}: renamed and described", st == 200 and json.loads(raw)["name"] == f"Shelf {sfx} B")
    check("…another member 403", _problem(ctx.request("PATCH", f"/api/v2/collections/{cid}", owner, {"name": "x"}), 403, "forbidden"))
    check("…a bad visibility 400 invalid_body",
          _problem(ctx.request("PATCH", f"/api/v2/collections/{cid}", member, {"visibility": "world"}), 400, "invalid_body"))
    priv = ctx.call("POST", "/api/v2/collections", member, {"name": f"Private Shelf {sfx}"}, expect=(201,))
    check("GET /collections/{id}: a private one is 404 to another member, even an admin",
          _problem(ctx.request("GET", f"/api/v2/collections/{priv['id']}", owner), 404, "not_found"))
    if outsider:
        check("…and to another household", _problem(ctx.request("GET", f"/api/v2/collections/{cid}", outsider), 404, "not_found"))
    for i in range(3):
        ctx.call("POST", "/api/v2/collections", member, {"name": f"Shelf {sfx} {i}"}, expect=(201,))
    full, _p = s.walk("/api/v2/collections", member, 500)
    small, pages = s.walk("/api/v2/collections", member, 2)
    check("GET /collections: paged two at a time without loss or repeats",
          [c["id"] for c in small] == [c["id"] for c in full] and pages > 1 and sum(c["name"].startswith("Shelf") for c in full) >= 4)
    feed = Feed(ctx, owner, "collection")
    check("DELETE /collections/{id}: a household admin on a member's shared one 204",
          ctx.status_of("DELETE", f"/api/v2/collections/{cid}", owner) == 204)
    check("…a removal in the feed", (r := _last(feed.read(), "collection", cid)) is not None and r["op"] == "delete")
    check("…a private one of someone else's 404 to an admin too",
          _problem(ctx.request("DELETE", f"/api/v2/collections/{priv['id']}", owner), 404, "not_found"))


def contributors(ctx: Ctx, s: Suite, owner: str, member: str, member_id: str) -> None:
    check = ctx.check
    sfx = ctx.sfx
    name = f"Writer Made {sfx}"
    key = {"Idempotency-Key": f"v2books-who-{uuid.uuid4()}"}
    first = ctx.request("POST", "/api/v2/contributors", member, {"name": name, "sort_name": f"Made, Writer {sfx}", "bio": "Wrote things."}, headers=key)
    made = json.loads(first[2]) if first[0] == 201 else {}
    check("POST /contributors: named with a sort name and a bio",
          first[0] == 201 and made.get("name") == name and made.get("bio") == "Wrote things." and "image_credit" in made, f"{first[0]} {first[2][:300]!r}")
    again = ctx.request("POST", "/api/v2/contributors", member, {"name": name, "sort_name": f"Made, Writer {sfx}", "bio": "Wrote things."}, headers=key)
    check("…a retry with the same Idempotency-Key answers the first", again[0] == 201 and json.loads(again[2])["id"] == made.get("id"))
    same = ctx.request("POST", "/api/v2/contributors", owner, {"name": name.upper()})
    check("…a name the household has answers that one, 200", same[0] == 200 and json.loads(same[2])["id"] == made.get("id"))
    _nid, noup = _no_upload(ctx, owner, "who-noup")
    check("…a member who may not add to the library 403", _problem(ctx.request("POST", "/api/v2/contributors", noup, {"name": "No"}), 403, "forbidden"))
    check("…an empty name 400 invalid_body", _problem(ctx.request("POST", "/api/v2/contributors", member, {"name": " "}), 400, "invalid_body"))
    kid = made["id"]
    st, _h, raw = ctx.request("PATCH", f"/api/v2/contributors/{kid}", owner, {"bio": "Wrote more."})
    check("PATCH /contributors/{id}: a bio for one credited nowhere yet", st == 200 and json.loads(raw).get("bio") == "Wrote more.", f"{st} {raw[:200]!r}")
    st, _h, raw = ctx.request("PATCH", f"/api/v2/contributors/{kid}", owner, {"bio": None})
    check("…null clears it", st == 200 and json.loads(raw).get("bio") is None)
    check("…too long 400 invalid_body", _problem(ctx.request("PATCH", f"/api/v2/contributors/{kid}", owner, {"bio": "x" * 10001}), 400, "invalid_body"))
    st, _h, raw = ctx.request("PUT", f"/api/v2/contributors/{kid}/image", member, {"upload_id": s.image(member, 30)})
    pic = json.loads(raw) if st == 200 else {}
    check("PUT /contributors/{id}/image: a photo set by hand, no credit to show",
          st == 200 and "/api/v2/images/" in (pic.get("image") or "") and pic.get("image_credit") is None, f"{st} {raw[:200]!r}")
    check("…not an image 400 not_an_image",
          _problem(ctx.request("PUT", f"/api/v2/contributors/{kid}/image", member,
                               {"upload_id": s.image(member, body=b"words", content_type="text/plain")}), 400, "not_an_image"))
    check("…a member who may not add to the library 403",
          _problem(ctx.request("PUT", f"/api/v2/contributors/{kid}/image", noup, {"upload_id": "x"}), 403, "forbidden"))
    check("DELETE /contributors/{id}/image: forgotten", ctx.status_of("DELETE", f"/api/v2/contributors/{kid}/image", member) == 204
          and ctx.call("PATCH", f"/api/v2/contributors/{kid}", member, {})["image"] is None)
    check("…unknown 404", _problem(ctx.request("DELETE", f"/api/v2/contributors/{uuid.uuid4()}/image", member), 404, "not_found"))
    # Credited: in use; uncredited: an admin removes it.
    b = s.book(owner, files=1)
    ctx.call("PUT", f"/api/v2/books/{b['id']}/credits", owner, {"credits": [{"contributor_id": kid, "role": "author"}]})
    check("GET /contributors/{id}: credited, every member finds them", ctx.status_of("GET", f"/api/v2/contributors/{kid}", member) == 200)
    check("DELETE /contributors/{id}: a member 403", _problem(ctx.request("DELETE", f"/api/v2/contributors/{kid}", member), 403, "forbidden"))
    check("…still credited 409 in_use", _problem(ctx.request("DELETE", f"/api/v2/contributors/{kid}", owner), 409, "in_use"))
    ctx.call("PUT", f"/api/v2/books/{b['id']}/credits", owner, {"credits": [{"name": f"Other Writer {sfx}", "role": "author"}]})
    check("…once nothing credits them, a household admin removes them",
          ctx.status_of("DELETE", f"/api/v2/contributors/{kid}", owner) == 204)
    check("…gone: 404", _problem(ctx.request("DELETE", f"/api/v2/contributors/{kid}", owner), 404, "not_found"))
    _ = member_id


def snapshot(ctx: Ctx, s: Suite, owner: str) -> None:
    check = ctx.check
    b = s.book(owner, files=1)
    made = ctx.call("POST", "/api/v2/series", owner, {"name": f"Snap Saga {ctx.sfx}", "books": [{"book_id": b["id"]}]}, expect=(201,))
    coll = ctx.call("POST", "/api/v2/collections", owner, {"name": f"Snap Shelf {ctx.sfx}", "book_ids": [b["id"]]}, expect=(201,))

    def lines(query: str) -> list[dict]:
        raw = ctx.call("GET", f"/api/v2/sync/snapshot{query}", owner, raw=True)
        return [json.loads(x) for x in raw.decode().splitlines() if x.strip()]

    picked = lines("?kinds=series,collection")
    recs = {(x["kind"], x["id"]): x for x in picked if x["type"] == "record"}
    check("GET /sync/snapshot?kinds=series,collection: the caller's series and collections, as their records",
          ("series", made["id"]) in recs and ("collection", coll["id"]) in recs
          and recs[("series", made["id"])]["record"]["books"][0]["book_id"] == b["id"]
          and recs[("collection", coll["id"])]["record"]["book_count"] == 1
          and {x["kind"] for x in picked if x["type"] == "record"} <= {"series", "collection"})
    ids = {(x["kind"], x["id"]) for x in lines("?kinds=series,collection&fields=ids") if x["type"] == "id"}
    check("…ids only: the same items", ids == set(recs))
    full = {(x.get("kind"), x.get("id")) for x in lines("") if x["type"] == "record"}
    check("…and in a snapshot of every kind", ("series", made["id"]) in full and ("collection", coll["id"]) in full)
