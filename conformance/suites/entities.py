# SPDX-License-Identifier: AGPL-3.0-or-later
"""Music entities (docs/data-model-plan.md): albums are rows the household owns, not GROUP BYs
over tag strings. Tracks on one album share its id however the tag is spelled; another album or
a retag moves a track to another id; two households never share one.
"""
from __future__ import annotations

import hashlib
import urllib.parse
import uuid

from core import Ctx

NAME = "entities"
PW = "EntitiesTest123!"


def run(ctx: Ctx) -> None:
    sfx = ctx.sfx
    _a_id, _a_email, tok = ctx.make_family_admin("a", PW)

    album = f"Entity Album {sfx}"
    one = ctx.upload_track(tok, f"One {sfx}", artist=f"Band {sfx}", album=album)
    two = ctx.upload_track(tok, f"Two {sfx}", artist=f"band {sfx} feat. Guest {sfx}", album=album.upper())
    other = ctx.upload_track(tok, f"Three {sfx}", artist=f"Band {sfx}", album=f"Other Album {sfx}")

    ctx.check("an uploaded track has an album id", bool(one.get("album_id")), str(one.get("album_id")))
    ctx.check("tracks on one album share its id, whatever the case of the tag and a guest on one of them",
              one.get("album_id") == two.get("album_id"), f"{one.get('album_id')} vs {two.get('album_id')}")
    ctx.check("another album is another id", other.get("album_id") not in (None, one.get("album_id")))

    edited = ctx.call("PUT", f"/api/v1/music/tracks/{two['id']}", tok,
                      {"title": two["title"], "artist": f"Band {sfx}", "album": f"Other Album {sfx}"})
    ctx.check("retagging a track moves it to the album its tags now name",
              edited.get("album_id") == other.get("album_id"), f"{edited.get('album_id')} vs {other.get('album_id')}")

    # Another household with an album of the same name keeps its own entity.
    if not ctx.feature("one_family"):
        _b_id, _b_email, b_tok = ctx.make_family_admin("b", PW)
        theirs = ctx.upload_track(b_tok, f"Theirs {sfx}", artist=f"Band {sfx}", album=album)
        ctx.check("two households never share an album", theirs.get("album_id") not in (None, one.get("album_id")))
    else:
        ctx.skip("two households", "one family per install")

    # The lists read the entities (E2): ids that match the tracks', artists by credit.
    duo = ctx.upload_track(tok, f"Duet {sfx}", artist=f"Solo {sfx}; Partner {sfx} feat. Visitor {sfx}", album=f"Duets {sfx}")
    artists = ctx.call("GET", "/api/v1/music/artists", tok)
    by_name = {a["artist"]: a for a in artists}
    ctx.check("artists carry ids", all(a.get("id") for a in artists))
    ctx.check("'A; B' is two artists", f"Solo {sfx}" in by_name and f"Partner {sfx}" in by_name,
              ", ".join(n for n in by_name if sfx in n))
    ctx.check("a guest after 'feat.' is not listed as an artist of the track", f"Visitor {sfx}" not in by_name)
    albums = ctx.call("GET", "/api/v1/music/albums", tok)
    duets = [a for a in albums if a.get("id") == duo.get("album_id")]
    ctx.check("the album list's id is the tracks' album_id", len(duets) == 1 and duets[0].get("track_count") == 1, str(duets)[:120])
    solo_id = by_name.get(f"Solo {sfx}", {}).get("id")
    if solo_id:
        mine = ctx.call("GET", f"/api/v1/music/albums?artist_id={solo_id}", tok)
        ctx.check("albums filter by artist id", [a["id"] for a in mine] == [duo["album_id"]], str([a.get("album") for a in mine]))

    # Subsonic speaks the same ids (E2b): no name-derived ids, stars and ratings on entities.
    key = ctx.call("GET", "/api/v1/users/me/subsonic-key", tok)
    salt = uuid.uuid4().hex[:8]
    auth = urllib.parse.urlencode({"u": key["username"], "t": hashlib.md5((key["api_key"] + salt).encode()).hexdigest(),
                                   "s": salt, "v": "1.16.1", "c": "entities", "f": "json"})

    def rest(endpoint: str, **params: str) -> dict:
        q = urllib.parse.urlencode(params)
        root = ctx.call("GET", f"/rest/{endpoint}.view?{auth}&{q}")["subsonic-response"]
        if root.get("status") != "ok":
            raise AssertionError(f"{endpoint}: {root.get('error')}")
        return root

    album_id = duo["album_id"]
    album = rest("getAlbum", id=album_id)["album"]
    ctx.check("Subsonic getAlbum takes the album's entity id", album.get("id") == album_id
              and [x.get("id") for x in album.get("song", [])] == [duo["id"]], str(album)[:120])
    ctx.check("a Subsonic song carries the entity ids", album["song"][0].get("albumId") == album_id
              and album["song"][0].get("artistId") == solo_id, str(album["song"][0])[:160])
    sub_artists = [a for i in rest("getArtists")["artists"].get("index", []) for a in i.get("artist", [])]
    ctx.check("Subsonic artist ids are the entity ids", any(a.get("id") == solo_id for a in sub_artists))
    ctx.check("Subsonic getArtist lists the artist's albums",
              [a.get("id") for a in rest("getArtist", id=solo_id)["artist"].get("album", [])] == [album_id])

    rest("star", albumId=album_id, artistId=solo_id)
    rest("setRating", id=album_id, rating="4")
    starred = rest("getStarred2")["starred2"]
    ctx.check("starring an album and an artist by id shows in getStarred2",
              any(a.get("id") == album_id for a in starred.get("album", []))
              and any(a.get("id") == solo_id for a in starred.get("artist", [])), str(starred)[:160])
    ctx.check("an album rating reads back on the album", rest("getAlbum", id=album_id)["album"].get("userRating") == 4)
    rest("unstar", albumId=album_id, artistId=solo_id)
    starred = rest("getStarred2")["starred2"]
    ctx.check("unstar removes both", not starred.get("album") and not starred.get("artist"), str(starred)[:120])
