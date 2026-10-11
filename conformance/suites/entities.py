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

    # Books credit the same contributors (E3): authors and narrators from the text fields,
    # "A & B" as two authors; changing a book's credits or tags is its owner's.
    book = ctx.call("POST", "/api/v1/audiobooks", tok,
                    {"title": f"Credits Book {sfx}", "author": f"Writer {sfx} & Cowriter {sfx}",
                     "narrator": f"Solo {sfx}", "visibility": "family"})
    credits = ctx.call("GET", f"/api/v1/audiobooks/authors/book/{book['id']}", tok)
    roles = sorted((c["role"], c["author_name"]) for c in credits)
    ctx.check("a book's author text becomes credits, 'A & B' as two authors",
              roles == sorted([("author", f"Writer {sfx}"), ("author", f"Cowriter {sfx}"), ("narrator", f"Solo {sfx}")]), str(roles))
    narrator = next((c for c in credits if c["role"] == "narrator"), {})
    ctx.check("the singer who narrates is one contributor", narrator.get("author_id") == solo_id,
              f"{narrator.get('author_id')} vs {solo_id}")
    authors = ctx.call("GET", "/api/v1/audiobooks/authors", tok)
    ctx.check("the author list holds the book's contributors", {f"Writer {sfx}", f"Solo {sfx}"} <= {a["name"] for a in authors})

    _m_id, _m_email, m_tok = ctx.make_user("member", PW)
    if not ctx.feature("one_family"):
        ctx.skip("member edits", "the member is not in the owner's household on this server")
    else:
        writer = next(c for c in credits if c["author_name"] == f"Writer {sfx}")
        ctx.call("POST", f"/api/v1/audiobooks/authors/book/{book['id']}", m_tok,
                 {"author_id": writer["author_id"], "role": "editor"}, expect=(403, 404))
        ctx.call("DELETE", f"/api/v1/audiobooks/authors/book/{book['id']}/{writer['author_id']}/author", m_tok, expect=(403, 404))
        ctx.call("PUT", f"/api/v1/audiobooks/authors/tags/book/{book['id']}", m_tok, {"tags": ["x"]}, expect=(403, 404))
        ctx.check("only a book's owner changes its credits and tags", True)
        ctx.call("DELETE", f"/api/v1/audiobooks/authors/{writer['author_id']}", m_tok, expect=(403,))
        ctx.check("deleting a contributor is a family admin's", True)
    ctx.call("POST", f"/api/v1/audiobooks/authors/book/{book['id']}", tok,
             {"author_id": solo_id, "role": "playwright"}, expect=(400,))
    ctx.check("a credit's role comes from the book roles", True)

    # Corrections (E4): fixed once on the entity, and they hold when the same tags are read again.
    def artist_named(name: str) -> dict | None:
        return next((a for a in ctx.call("GET", "/api/v1/music/artists", tok) if a["artist"] == name), None)

    duo_tag = f"Hall {sfx} & Oates {sfx}"
    first = ctx.upload_track(tok, f"Rich Girl {sfx}", artist=duo_tag, album=f"Duo Album {sfx}")
    combined = artist_named(duo_tag)
    parts = ctx.call("POST", f"/api/v1/music/artists/{combined['id']}/split", tok,
                     {"names": [f"Daryl {sfx}", f"John {sfx}"]})
    ctx.check("a split answers with the artists, in order",
              [p["name"] for p in parts] == [f"Daryl {sfx}", f"John {sfx}"], str(parts))
    ctx.check("the split artist is gone from the list", artist_named(duo_tag) is None)
    ctx.upload_track(tok, f"Maneater {sfx}", artist=duo_tag, album=f"Duo Album {sfx}")
    ctx.call("PUT", f"/api/v1/music/tracks/{first['id']}", tok,
             {"title": first["title"], "artist": duo_tag, "album": f"Duo Album {sfx}"})
    john = artist_named(f"John {sfx}")
    ctx.check("a new file and a retag with the split name credit every part",
              artist_named(duo_tag) is None and john is not None and john["track_count"] == 2, str(john))
    duo_albums = ctx.call("GET", f"/api/v1/music/albums?artist_id={parts[0]['id']}", tok)
    ctx.check("the album credits the parts too, and reads as them",
              [a["artist"] for a in duo_albums] == [f"Daryl {sfx}; John {sfx}"], str(duo_albums)[:160])

    renamed = ctx.call("PATCH", f"/api/v1/music/artists/{parts[0]['id']}", tok, {"name": f"Daryl Hall {sfx}"})
    ctx.check("renaming an artist answers with the new name", renamed.get("name") == f"Daryl Hall {sfx}", str(renamed))
    ctx.upload_track(tok, f"Sara Smile {sfx}", artist=f"Daryl {sfx}", album=f"Solo Album {sfx}")
    daryl = artist_named(f"Daryl Hall {sfx}")
    ctx.check("the old spelling still finds the renamed artist",
              artist_named(f"Daryl {sfx}") is None and daryl is not None and daryl["track_count"] == 3, str(daryl))
    ctx.call("PATCH", f"/api/v1/music/artists/{parts[1]['id']}", tok, {"name": f"daryl hall {sfx}"}, expect=(409,))
    ctx.check("a name another artist has is a merge, not a rename", True)

    ctx.upload_track(tok, f"Help {sfx}", artist=f"Beatles {sfx}", album=f"Abbey {sfx}")
    ctx.upload_track(tok, f"Come Together {sfx}", artist=f"The Beatles {sfx}", album=f"Abbey {sfx} (Remastered)")
    beatles, the_beatles = artist_named(f"Beatles {sfx}"), artist_named(f"The Beatles {sfx}")
    kept = ctx.call("POST", f"/api/v1/music/artists/{beatles['id']}/merge", tok, {"into": the_beatles["id"]})
    ctx.check("a merge answers with the artist that stays", kept.get("id") == the_beatles["id"], str(kept))
    ctx.upload_track(tok, f"Yesterday {sfx}", artist=f"Beatles {sfx}", album=f"Help {sfx}")
    merged = artist_named(f"The Beatles {sfx}")
    ctx.check("a merged artist's spelling finds the one that stays",
              artist_named(f"Beatles {sfx}") is None and merged is not None and merged["track_count"] == 3, str(merged))

    albums = {a["album"]: a for a in ctx.call("GET", f"/api/v1/music/albums?artist_id={the_beatles['id']}", tok)}
    abbey, remaster = albums[f"Abbey {sfx}"], albums[f"Abbey {sfx} (Remastered)"]
    ctx.call("PUT", f"/api/v1/music/albums/{remaster['id']}/release-group", tok,
             {"release_group_id": abbey["release_group_id"]})
    albums = {a["album"]: a for a in ctx.call("GET", f"/api/v1/music/albums?artist_id={the_beatles['id']}", tok)}
    ctx.check("two editions grouped share a release group",
              albums[f"Abbey {sfx} (Remastered)"]["release_group_id"] == abbey["release_group_id"])
    ctx.call("POST", f"/api/v1/music/albums/{remaster['id']}/merge", tok, {"into": abbey["id"]}, expect=(204,))
    again = ctx.upload_track(tok, f"Something {sfx}", artist=f"The Beatles {sfx}", album=f"Abbey {sfx} (Remastered)")
    ctx.check("a merged album's tags lead to the one that stays", again.get("album_id") == abbey["id"],
              f"{again.get('album_id')} vs {abbey['id']}")
    ctx.call("PATCH", f"/api/v1/music/albums/{abbey['id']}", tok, {"title": f"Abbey Road {sfx}"}, expect=(204,))
    retagged = ctx.call("PUT", f"/api/v1/music/tracks/{again['id']}", tok,
                        {"title": again["title"], "artist": f"The Beatles {sfx}", "album": f"Abbey {sfx}"})
    titles = [a["album"] for a in ctx.call("GET", f"/api/v1/music/albums?artist_id={the_beatles['id']}", tok)]
    ctx.check("a renamed album keeps its tracks and its title through a retag",
              retagged.get("album_id") == abbey["id"] and f"Abbey Road {sfx}" in titles and f"Abbey {sfx}" not in titles,
              str(titles))

    ctx.call("POST", f"/api/v1/music/artists/{the_beatles['id']}/merge", m_tok, {"into": parts[1]["id"]}, expect=(403, 404))
    ctx.call("POST", f"/api/v1/music/artists/{the_beatles['id']}/split", m_tok, {"names": ["x", "y"]}, expect=(403, 404))
    ctx.call("POST", f"/api/v1/music/albums/{abbey['id']}/merge", m_tok, {"into": abbey["id"]}, expect=(403, 404))
    ctx.check("merging and splitting are a family admin's", True)
