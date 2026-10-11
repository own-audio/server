# SPDX-License-Identifier: AGPL-3.0-or-later
"""Music entities (docs/data-model-plan.md): albums are rows the household owns, not GROUP BYs
over tag strings. Tracks on one album share its id however the tag is spelled; another album or
a retag moves a track to another id; two households never share one.
"""
from __future__ import annotations

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
