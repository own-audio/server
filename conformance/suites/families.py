# SPDX-License-Identifier: AGPL-3.0-or-later
"""Private/family folder model: who can see, stream and edit shared content.

Builds a family of three (owner, adult, kid), uploads a track privately,
shares it, and asserts who can see and stream it at each step. Then the same
for audiobooks, parental policies and grants, search and the private folder,
unsharing, and what happens to shared content when the owner leaves.
"""
from __future__ import annotations

from core import Ctx

NAME = "families"
PW = "FamilyTest123!"


def _has(items, key, value) -> bool:
    return any(i.get(key) == value for i in items)


def run(ctx: Ctx) -> None:
    sfx = ctx.sfx
    call, check = ctx.call, ctx.check

    owner_id, owner_email, owner_tok = ctx.make_family_admin("owner", PW)
    one_family = ctx.feature("one_family")
    adult_id, adult_email, adult_tok = ctx.make_user("adult", PW)
    kid_id, kid_email, kid_tok = ctx.make_user("kid", PW)

    ctx.log("\n[setup] owner invites adult + kid into their family")
    for email, tok in ((adult_email, adult_tok), (kid_email, kid_tok)):
        inv = call("POST", "/api/v1/family/invites", owner_tok, {"email": email, "role": "member"})
        call("POST", "/api/v1/family/invites/accept", tok, {"code": inv["code"]})
    fam = call("GET", "/api/v1/family", owner_tok)
    ids = {m["user_id"] for m in fam["members"]}
    if one_family:
        check("all three are in the install's family", {owner_id, adult_id, kid_id} <= ids)
    else:
        check("family has 3 members", len(fam["members"]) == 3, f"members={len(fam['members'])}")

    ctx.log("\n[private folder]")
    audio = b"ID3" + b"\x00" * 2048
    track = call("POST", "/api/v1/music/tracks/upload", owner_tok, multipart={
        "title": f"Private Song {sfx}", "artist": "Owner",
        "visibility": "private", "file": ("private.mp3", audio)})
    tid = track["id"]
    check("upload reports visibility=private", track["visibility"] == "private", track["visibility"])
    check("owner is marked as owner", track["is_owner"] is True)

    check("owner sees it in their list", _has(call("GET", "/api/v1/music/tracks", owner_tok), "id", tid))
    check("FAMILY ADMIN'S OTHER MEMBERS DO NOT SEE IT",
          not _has(call("GET", "/api/v1/music/tracks", adult_tok), "id", tid))
    check("kid does not see it", not _has(call("GET", "/api/v1/music/tracks", kid_tok), "id", tid))
    call("GET", f"/api/v1/music/tracks/{tid}", adult_tok, expect=(401, 403, 404))
    check("adult cannot fetch it directly", True)
    call("GET", f"/api/v1/music/tracks/{tid}/stream", adult_tok, expect=(401, 403, 404))
    check("adult cannot stream it", True)

    ctx.log("\n[shared folder]")
    shared = call("PUT", f"/api/v1/music/tracks/{tid}/visibility", owner_tok, {"visibility": "family"})
    check("visibility flips to family", shared["visibility"] == "family", shared["visibility"])
    check("adult now sees it", _has(call("GET", "/api/v1/music/tracks", adult_tok), "id", tid))
    check("kid now sees it", _has(call("GET", "/api/v1/music/tracks", kid_tok), "id", tid))
    adult_view = call("GET", f"/api/v1/music/tracks/{tid}", adult_tok)
    check("adult is NOT marked owner", adult_view["is_owner"] is False)
    call("GET", f"/api/v1/music/tracks/{tid}/stream", adult_tok, raw=True)
    check("adult can stream it", True)

    # Non-owners must not be able to edit or delete shared content.
    call("PUT", f"/api/v1/music/tracks/{tid}", adult_tok, {"title": "hijacked"}, expect=(401, 403, 404))
    check("adult cannot edit a shared track they do not own", True)
    call("DELETE", f"/api/v1/music/tracks/{tid}", adult_tok, expect=(401, 403, 404))
    check("adult cannot delete it", True)
    call("PUT", f"/api/v1/music/tracks/{tid}/visibility", adult_tok, {"visibility": "private"}, expect=(401, 403, 404))
    check("adult cannot change its visibility", True)

    ctx.log("\n[audiobooks]")
    book = call("POST", "/api/v1/audiobooks", owner_tok,
                {"title": f"Private Book {sfx}", "author": "Owner", "visibility": "private"})
    bid = book["id"]
    check("book created private", book["visibility"] == "private", book["visibility"])
    check("adult cannot see private book", not _has(call("GET", "/api/v1/audiobooks", adult_tok), "id", bid))
    call("PUT", f"/api/v1/audiobooks/{bid}/visibility", owner_tok, {"visibility": "family"})
    check("adult sees shared book", _has(call("GET", "/api/v1/audiobooks", adult_tok), "id", bid))

    ctx.log("\n[parental control]")
    call("PUT", f"/api/v1/family/members/{kid_id}/policy", owner_tok,
         {"media_kind": "audiobook", "policy": "deny_all"})
    check("kid loses shared books after deny_all", not _has(call("GET", "/api/v1/audiobooks", kid_tok), "id", bid))
    check("adult is unaffected", _has(call("GET", "/api/v1/audiobooks", adult_tok), "id", bid))

    call("PUT", f"/api/v1/family/members/{kid_id}/grants", owner_tok, {"media_kind": "audiobook", "allow": [bid]})
    check("explicit allow grant restores this one book", _has(call("GET", "/api/v1/audiobooks", kid_tok), "id", bid))

    # Only this suite's members: on a one-family server the install's admin is here too.
    ours = (owner_id, adult_id, kid_id)
    audience = [a for a in call("GET", f"/api/v1/family/content/audiobook/{bid}/audience", owner_tok)
                if a["user_id"] in ours]
    check("audience lists all three members", len(audience) == 3, f"audience={len(audience)}")
    check("audience shows everyone can listen", all(a["can_listen"] for a in audience))

    call("PUT", f"/api/v1/family/members/{kid_id}/grants", owner_tok, {"media_kind": "audiobook", "deny": [bid]})
    check("deny grant blocks the kid again", not _has(call("GET", "/api/v1/audiobooks", kid_tok), "id", bid))
    audience = [a for a in call("GET", f"/api/v1/family/content/audiobook/{bid}/audience", owner_tok)
                if a["user_id"] in ours]
    denied = [a for a in audience if not a["can_listen"]]
    check("audience reflects the denial", len(denied) == 1 and denied[0]["user_id"] == kid_id,
          f"denied={[a['user_id'] for a in denied]}")

    call("GET", f"/api/v1/family/members/{adult_id}/access", kid_tok, expect=(401, 403, 404))
    check("kid cannot read another member's restrictions", True)
    call("PUT", f"/api/v1/family/members/{kid_id}/policy", kid_tok,
         {"media_kind": "audiobook", "policy": "allow_all"}, expect=(401, 403))
    check("kid cannot lift their own restriction", True)

    ctx.log("\n[search + private folder]")
    hits = call("GET", f"/api/v1/library/search?q=Private%20Book%20{sfx}", adult_tok)
    check("adult finds the shared book in search", _has(hits, "id", bid))
    # A second track that is never shared, so the private folder has content
    # to list at this point (tid and bid are both shared right now).
    kept = call("POST", "/api/v1/music/tracks/upload", owner_tok, multipart={
        "title": f"Kept Private {sfx}", "artist": "Owner",
        "visibility": "private", "file": ("kept.mp3", audio)})
    priv = call("GET", "/api/v1/library/private", owner_tok)
    check("owner's private folder lists the never-shared track", _has(priv, "id", kept["id"]))
    check("shared track is not in the private folder", not _has(priv, "id", tid))
    check("shared book is not in the private folder", not _has(priv, "id", bid))
    check("private track stays out of another member's search",
          not _has(call("GET", f"/api/v1/library/search?q=Kept%20Private%20{sfx}", adult_tok), "id", kept["id"]))
    adult_priv = call("GET", "/api/v1/library/private", adult_tok)
    check("adult's private folder is empty", adult_priv == [], f"{adult_priv!r}"[:200])

    ctx.log("\n[unshare]")
    back = call("PUT", f"/api/v1/music/tracks/{tid}/visibility", owner_tok, {"visibility": "private"})
    check("visibility flips back to private", back["visibility"] == "private", back["visibility"])
    check("adult loses access again", not _has(call("GET", "/api/v1/music/tracks", adult_tok), "id", tid))

    ctx.log("\n[leaving the family]")
    call("PUT", f"/api/v1/music/tracks/{tid}/visibility", owner_tok, {"visibility": "family"})
    check("re-shared before departure", _has(call("GET", "/api/v1/music/tracks", adult_tok), "id", tid))
    if one_family:
        call("DELETE", f"/api/v1/family/members/{owner_id}", owner_tok, expect=(409,))
        check("nobody leaves the only family (409)", True)
        call("DELETE", f"/api/v1/family/members/{adult_id}", owner_tok, expect=(409,))
        check("nor is anyone removed into a second one", True)
        return
    # The last family admin may not leave, so hand the role over first.
    call("DELETE", f"/api/v1/family/members/{owner_id}", owner_tok, expect=(400,))
    check("last family admin is blocked from leaving", True)
    call("PUT", f"/api/v1/family/members/{adult_id}", owner_tok, {"role": "family_admin"})
    call("DELETE", f"/api/v1/family/members/{owner_id}", owner_tok)
    check("adult loses access after the owner leaves",
          not _has(call("GET", "/api/v1/music/tracks", adult_tok), "id", tid))
    owner_track = call("GET", f"/api/v1/music/tracks/{tid}", owner_tok)
    check("owner keeps it, now private", owner_track["visibility"] == "private", owner_track["visibility"])
