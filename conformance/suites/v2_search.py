# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 search across media and the lists' owner and visibility filters (completion plan phase 10).

`GET /search` with `types=books,podcasts,episodes,contributors`: each found by a part of its
title or a credited name, any case, only what the caller can see (another member's private
book, show and episode stay hidden; the owner finds their own), titles that start with the
words first, `include=contributors` embedding the books' people, the default still music only,
`invalid_type`. `owner=me` and `visibility=private|family` on `GET /tracks`, `/albums`,
`/books`, `/podcasts`: what each keeps, both together (v1's `GET /library/private`), another
member's items never widened into view, `invalid_owner` and `invalid_visibility`, and each
filtered list paged two at a time against one long page without loss or repeats.
"""
from __future__ import annotations

import json
import urllib.parse

from core import Ctx
from suites.v2_books import _problem
from suites.v2_sync import _upload

NAME = "v2_search"
PW = "V2SearchTest123!"


def _q(path: str, **kw) -> str:
    return path + "?" + urllib.parse.urlencode({k: v for k, v in kw.items() if v is not None})


def _ids(items: list[dict]) -> list[str]:
    return [i["id"] for i in items]


def _walk(ctx: Ctx, token: str, path: str, limit: int) -> tuple[list[str], int]:
    items, cursor, pages = [], None, 0
    while True:
        url = path + f"&limit={limit}" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else "")
        page = ctx.call("GET", url, token)
        items += _ids(page["items"])
        cursor, pages = page.get("next"), pages + 1
        if not cursor or pages > 200:
            return items, pages


def run(ctx: Ctx) -> None:
    check, sfx = ctx.check, ctx.sfx
    one_household = ctx.feature("one_family")
    owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    member_id, member_email, member = ctx.make_user("member", PW)
    if not one_household:
        inv = ctx.call("POST", "/api/v2/family/invites", owner, {"kind": "email", "email": member_email, "role": "member"}, expect=(201,))
        ctx.call("POST", "/api/v2/family/join", member, {"code": inv["code"]})

    word = f"Qx{sfx}"
    host = f"Hostname {word}"
    writer = f"Writer {word}"

    def book(token: str, title: str, visibility: str, author: str | None = None) -> dict:
        return ctx.call("POST", "/api/v2/books", token, {
            "title": title, "author": author or writer, "narrator": f"Reader {word}", "visibility": visibility,
            "files": [{"upload_id": _upload(ctx, token, "book_file", "s.mp3"), "relative_path": "01.mp3"}]}, expect=(201,))

    def show(token: str, title: str, visibility: str) -> dict:
        made = ctx.call("POST", "/api/v2/podcasts", token, {"title": title, "author": host, "visibility": visibility,
                                                            "guid": f"search-{title}"}, expect=(200, 201))
        return made

    def episode(token: str, show_id: str, title: str) -> dict:
        return ctx.call("POST", f"/api/v2/podcasts/{show_id}/episodes", token, {
            "upload_id": _upload(ctx, token, "episode", "e.mp3"), "guid": f"search-{title}", "title": title}, expect=(200, 201))

    shared_book = book(owner, f"{word} Shared Book", "family")
    later_book = book(owner, f"Also {word} Book", "family")
    private_book = book(owner, f"{word} Private Book", "private")
    members_book = book(member, f"{word} Member Private Book", "private", author=f"Hidden Writer {sfx}")
    shared_show = show(owner, f"{word} Shared Show", "family")
    private_show = show(owner, f"{word} Private Show", "private")
    members_show = show(member, f"{word} Member Show", "private")
    shared_ep = episode(owner, shared_show["id"], f"{word} Shared Episode")
    private_ep = episode(owner, private_show["id"], f"{word} Private Episode")
    members_ep = episode(member, members_show["id"], f"{word} Member Episode")
    tracks = {
        "own_private": [ctx.upload_track(owner, f"{word} Song P{i}", artist=f"Band {word}", album=f"Album P {word}", visibility="private",
                                         content=ctx.tone(), content_type="audio/wav") for i in range(3)],
        "own_shared": [ctx.upload_track(owner, f"{word} Song F{i}", artist=f"Band {word}", album=f"Album F {word}", visibility="family",
                                        content=ctx.tone(), content_type="audio/wav") for i in range(3)],
        "member_private": [ctx.upload_track(member, f"{word} Song M{i}", artist=f"Band {word}", album=f"Album M {word}", visibility="private",
                                            content=ctx.tone(), content_type="audio/wav") for i in range(2)],
        "member_shared": [ctx.upload_track(member, f"{word} Song S{i}", artist=f"Band {word}", album=f"Album S {word}", visibility="family",
                                           content=ctx.tone(), content_type="audio/wav") for i in range(2)],
    }

    # ── Search across media ────────────────────────────────────────────────
    every = "books,podcasts,episodes,contributors"
    as_member = ctx.call("GET", _q("/api/v2/search", q=word.lower(), types=every, limit=50), member)
    check("search books: the household's shared ones and the member's own, any case",
          {shared_book["id"], later_book["id"], members_book["id"]} <= set(_ids(as_member["books"])), str(_ids(as_member["books"])))
    check("…never another member's private book", private_book["id"] not in _ids(as_member["books"]))
    check("…titles that start with the words first",
          _ids(as_member["books"]).index(shared_book["id"]) < _ids(as_member["books"]).index(later_book["id"]))
    check("search podcasts: shared and own, not another's private", shared_show["id"] in _ids(as_member["podcasts"])
          and members_show["id"] in _ids(as_member["podcasts"]) and private_show["id"] not in _ids(as_member["podcasts"]),
          str(_ids(as_member["podcasts"])))
    check("search episodes: of visible shows only", shared_ep["id"] in _ids(as_member["episodes"])
          and members_ep["id"] in _ids(as_member["episodes"]) and private_ep["id"] not in _ids(as_member["episodes"]),
          str(_ids(as_member["episodes"])))
    check("…an episode found is the episode record", all("podcast_id" in e and "title" in e for e in as_member["episodes"]))
    names = [c["name"] for c in as_member["contributors"]]
    check("search contributors: authors, narrators, hosts and artists by name", writer in names and f"Reader {word}" in names
          and host in names and f"Band {word}" in names, str(names))
    check("…music stays out of a search that did not ask for it",
          as_member["tracks"] == [] and as_member["albums"] == [] and as_member["artists"] == [])
    hidden = ctx.call("GET", _q("/api/v2/search", q=f"Hidden Writer {sfx}", types="contributors,books"), owner)
    check("…someone credited only on another member's private book is not found", hidden["contributors"] == [] and hidden["books"] == [],
          json.dumps(hidden)[:300])
    own = ctx.call("GET", _q("/api/v2/search", q=word, types="books,podcasts,episodes", limit=50), owner)
    check("the owner finds their own private book, show and episode", private_book["id"] in _ids(own["books"])
          and private_show["id"] in _ids(own["podcasts"]) and private_ep["id"] in _ids(own["episodes"]))
    by_writer = ctx.call("GET", _q("/api/v2/search", q=writer, types="books", include="contributors"), member)
    check("a book found by its author's name", shared_book["id"] in _ids(by_writer["books"]), str(_ids(by_writer["books"])))
    embedded = {c["id"] for c in (by_writer.get("included") or {}).get("contributors", [])}
    credited = {c["contributor_id"] for b in by_writer["books"] for c in b["credits"]}
    check("…include=contributors embeds the books' people once", credited and credited <= embedded, f"{credited} {embedded}")
    by_host = ctx.call("GET", _q("/api/v2/search", q=host, types="podcasts"), member)
    check("a show found by its host's name", shared_show["id"] in _ids(by_host["podcasts"]))
    artists_only = ctx.call("GET", _q("/api/v2/search", q=writer, types="artists"), member)
    check("an author is a contributor, not an artist", artists_only["artists"] == [], str(artists_only["artists"]))
    default = ctx.call("GET", _q("/api/v2/search", q=word), member)
    check("without types: artists, albums and tracks, as before", default["tracks"] and default["books"] == []
          and default["podcasts"] == [] and default["episodes"] == [] and default["contributors"] == [],
          json.dumps({k: len(v) for k, v in default.items() if isinstance(v, list)}))
    check("search limit=1 keeps one of each", all(len(v) <= 1 for k, v in
                                                     ctx.call("GET", _q("/api/v2/search", q=word, types=every, limit=1), member).items()
                                                     if isinstance(v, list)))
    check("search types=films: 400 invalid_type", _problem(ctx.request("GET", _q("/api/v2/search", q=word, types="books,films"), member), 400, "invalid_type"))

    # ── Owner and visibility filters ───────────────────────────────────────
    def listed(path: str, token: str, **kw) -> set[str]:
        ids: set[str] = set()
        url, cursor = _q(path, q=word, limit=500, **kw), None
        while True:
            page = ctx.call("GET", url + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else ""), token)
            ids |= set(_ids(page["items"]))
            cursor = page.get("next")
            if not cursor:
                return ids

    t = {k: {x["id"] for x in v} for k, v in tracks.items()}
    seen_by_member = t["member_private"] | t["member_shared"] | t["own_shared"]
    check("GET /tracks?owner=me: only the caller's own", listed("/api/v2/tracks", member, owner="me") == t["member_private"] | t["member_shared"])
    check("GET /tracks?visibility=private: only what the caller keeps private",
          listed("/api/v2/tracks", member, visibility="private") == t["member_private"])
    check("GET /tracks?visibility=family: the household's shared ones, the caller's included",
          listed("/api/v2/tracks", member, visibility="family") == t["member_shared"] | t["own_shared"])
    check("GET /tracks?owner=me&visibility=private: the private folder (v1's /library/private)",
          listed("/api/v2/tracks", owner, owner="me", visibility="private") == t["own_private"])
    check("…the filters only narrow: no filter is everything visible", listed("/api/v2/tracks", member) == seen_by_member)

    albums_mine = listed("/api/v2/albums", member, owner="me")
    albums_private = listed("/api/v2/albums", member, visibility="private")
    albums_all = listed("/api/v2/albums", member)
    check("GET /albums?owner=me and visibility=private: albums with such a track",
          len(albums_mine) == 2 and len(albums_private) == 1 and albums_private < albums_mine < albums_all and len(albums_all) == 3,
          f"{len(albums_mine)} {len(albums_private)} {len(albums_all)}")
    check("GET /books?owner=me", listed("/api/v2/books", member, owner="me") == {members_book["id"]})
    check("GET /books?visibility=family", listed("/api/v2/books", member, visibility="family") == {shared_book["id"], later_book["id"]})
    check("GET /books?owner=me&visibility=private as the owner", listed("/api/v2/books", owner, owner="me", visibility="private") == {private_book["id"]})
    check("GET /podcasts?owner=me", listed("/api/v2/podcasts", member, owner="me") == {members_show["id"]})
    check("GET /podcasts?visibility=private as the owner", listed("/api/v2/podcasts", owner, visibility="private") == {private_show["id"]})
    check("GET /podcasts?visibility=private as the member: another's private show stays hidden",
          private_show["id"] not in listed("/api/v2/podcasts", member, visibility="private"))
    for path in ("/api/v2/tracks", "/api/v2/albums", "/api/v2/books", "/api/v2/podcasts"):
        check(f"GET {path.removeprefix('/api/v2')}?owner=someone: 400 invalid_owner",
              _problem(ctx.request("GET", _q(path, owner="someone"), member), 400, "invalid_owner"))
        check(f"GET {path.removeprefix('/api/v2')}?visibility=shared: 400 invalid_visibility",
              _problem(ctx.request("GET", _q(path, visibility="shared"), member), 400, "invalid_visibility"))

    # Paging without loss with the filters on, in each order.
    for path, kw, sorts in (("/api/v2/tracks", {"owner": "me"}, [None, "added"]),
                            ("/api/v2/tracks", {"visibility": "family"}, [None, "added"]),
                            ("/api/v2/books", {"visibility": "family"}, [None, "added"]),
                            ("/api/v2/albums", {"owner": "me"}, [None, "added"])):
        for sort in sorts:
            base = _q(path, q=word, sort=sort, **kw)
            full, _ = _walk(ctx, member, base, 500)
            small, pages = _walk(ctx, member, base, 1)
            check(f"paging {path.removeprefix('/api/v2')} {kw}" + (f" sort={sort}" if sort else "") + " one at a time without loss",
                  small == full and len(set(small)) == len(small) and len(full) >= 2 and pages > 1, f"{len(small)} vs {len(full)} in {pages}")
