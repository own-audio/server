# SPDX-License-Identifier: AGPL-3.0-or-later
"""The metadata organizer and the edit history (revision 15,
docs/metadata-organizer-plan.md O0–O1): the songs as a table with filters and
facets, a save of many songs as one batch, the history of it, and undo — all
or nothing, refused when the fields changed again since.

Runs on the fixture library (tools/make_library_fixtures.sh), whose songs the
admin owns; it leaves them as it found them.
"""

NAME = "organize"


def run(ctx):
    if (ctx.server.get("api") or {}).get("revision", 0) < 15:
        ctx.skip("organizer", "not offered by this server")
        return
    facets = ctx.call("GET", "/api/v2/organize/facets", ctx.admin_token)
    counts = facets.get("counts") or {}
    ctx.check("facets count the songs and the quick filters",
              counts.get("all", 0) >= 3 and all(k in counts for k in ("unidentified", "no_cover", "no_year", "no_genre", "edited")),
              str(counts))
    folder = next((f for f in facets.get("folders", []) if f.get("id")), None)
    if not folder:
        ctx.skip("organizer", "no library folder with songs here")
        return
    page = ctx.call("GET", f"/api/v2/organize/tracks?folder={folder['id']}&path=Test%20Artist/Test%20Album/&sort=track", ctx.admin_token)
    songs = page.get("items", [])
    # The library suite may have hidden one of the three; what is left is in order.
    numbers = [s["track_number"] for s in songs]
    ctx.check("a folder path narrows the list to the album, in track order",
              2 <= len(songs) <= 3 and numbers == sorted(numbers) and page.get("total") == len(songs)
              and all(s["album"] == "Test Album" for s in songs), str([s["title"] for s in songs]))
    ctx.check("songs say where their file is", all(s["path"].startswith("Test Artist/Test Album/") for s in songs),
              str([s["path"] for s in songs]))
    ctx.call("GET", "/api/v2/organize/tracks?sort=nonsense", ctx.admin_token, expect=(400,))
    word = songs[-1]["title"] if songs else "Song"
    found = ctx.call("GET", f"/api/v2/organize/tracks?q={word.replace(' ', '%20')}", ctx.admin_token).get("items", [])
    ctx.check("words find a song", [s["title"] for s in found] == [word], str([s["title"] for s in found]))
    two = ctx.call("GET", "/api/v2/organize/tracks?limit=2&sort=title", ctx.admin_token)
    rest = ctx.call("GET", f"/api/v2/organize/tracks?limit=2&sort=title&cursor={two['next']}", ctx.admin_token) if two.get("next") else {}
    ctx.check("pages follow on", two.get("next") and not ({s["id"] for s in two["items"]} & {s["id"] for s in rest.get("items", [])}))

    if len(songs) < 2:
        return
    a, b = songs[0], songs[1]
    ctx.call("PATCH", "/api/v2/tracks", ctx.admin_token, body={"tracks": [{"id": a["id"], "title": " "}]}, expect=(400,))
    saved = ctx.call("PATCH", "/api/v2/tracks", ctx.admin_token,
                     body={"tracks": [{"id": a["id"], "genre": "Organizer Test"}, {"id": b["id"], "genre": "Organizer Test"}]})
    ctx.check("a save changes the songs as one batch", saved.get("updated") == 2 and saved.get("batch_id"), str(saved))
    batch = saved["batch_id"]
    edits = ctx.call("GET", f"/api/v2/edits?batch_id={batch}", ctx.admin_token).get("items", [])
    ctx.check("the history keeps each change, before and after",
              len(edits) == 2 and all(e["after"] == {"genre": "Organizer Test"} and "genre" in e["before"]
                                     and e["source"] == "bulk" for e in edits), str(edits)[:300])
    batches = ctx.call("GET", "/api/v2/edits/batches?limit=5", ctx.admin_token).get("items", [])
    ctx.check("the save is one entry in the history list",
              batches and batches[0]["batch_id"] == batch and batches[0]["items"] == 2, str(batches[:1]))
    edited = ctx.call("GET", "/api/v2/organize/tracks?edited=true", ctx.admin_token).get("items", [])
    ctx.check("edited songs can be listed", {a["id"], b["id"]} <= {s["id"] for s in edited})

    # Someone else sees none of it and changes none of it.
    _, _, other = ctx.make_user("organizer", "Organizer123!-long")
    theirs = ctx.call("PATCH", "/api/v2/tracks", other, body={"tracks": [{"id": a["id"], "genre": "Not yours"}]})
    ctx.check("a save leaves other people's songs alone", theirs.get("updated") == 0, str(theirs))
    ctx.call("POST", f"/api/v2/edits/batches/{batch}/undo", other, expect=(404,))
    ctx.check("other people's history is not shown",
              not ctx.call("GET", f"/api/v2/edits?batch_id={batch}", other).get("items"))

    # A later change to the same field: the earlier one cannot be undone over it.
    later = ctx.call("PATCH", "/api/v2/tracks", ctx.admin_token, body={"tracks": [{"id": a["id"], "genre": "Later"}]})
    ctx.call("POST", f"/api/v2/edits/batches/{batch}/undo", ctx.admin_token, expect=(409,))
    still = {s["id"]: s for s in ctx.call("GET", "/api/v2/organize/tracks?edited=true", ctx.admin_token)["items"]}
    ctx.check("a refused undo changes nothing",
              still[a["id"]]["genre"] == "Later" and still[b["id"]]["genre"] == "Organizer Test",
              str((still[a["id"]]["genre"], still[b["id"]]["genre"])))
    one = ctx.call("GET", f"/api/v2/edits?batch_id={later['batch_id']}", ctx.admin_token)["items"][0]
    undone = ctx.call("POST", f"/api/v2/edits/{one['id']}/undo", ctx.admin_token)
    ctx.check("one change is undone", undone.get("items") == 1, str(undone))
    ctx.call("POST", f"/api/v2/edits/{one['id']}/undo", ctx.admin_token, expect=(409,))
    back = ctx.call("POST", f"/api/v2/edits/batches/{batch}/undo", ctx.admin_token)
    ctx.check("then the earlier save, all of it", back.get("items") == 2, str(back))
    now = {s["id"]: s for s in ctx.call("GET", f"/api/v2/organize/tracks?folder={folder['id']}&path=Test%20Artist/Test%20Album/", ctx.admin_token)["items"]}
    ctx.check("the songs are as they were", now[a["id"]]["genre"] == a["genre"] and now[b["id"]]["genre"] == b["genre"],
              str((now[a["id"]]["genre"], a["genre"])))
    undo_entry = ctx.call("GET", "/api/v2/edits/batches?limit=1", ctx.admin_token)["items"][0]
    ctx.check("an undo is in the history too", undo_entry["source"] == "undo", str(undo_entry))

    # An album's change is kept and undone the same way.
    if a.get("album_id"):
        album = a["album_id"]
        before = ctx.call("GET", f"/api/v2/albums/{album}", ctx.admin_token)
        ctx.call("PATCH", f"/api/v2/albums/{album}", ctx.admin_token, body={"release_date": "1999"})
        hist = ctx.call("GET", f"/api/v2/edits?item_kind=album&item_id={album}", ctx.admin_token)["items"]
        ctx.check("an album's change is in its history", hist and hist[0]["after"].get("release_date") == "1999", str(hist[:1]))
        ctx.call("POST", f"/api/v2/edits/{hist[0]['id']}/undo", ctx.admin_token)
        after = ctx.call("GET", f"/api/v2/albums/{album}", ctx.admin_token)
        ctx.check("and undone", after.get("release_date") == before.get("release_date"),
                  str((after.get("release_date"), before.get("release_date"))))
    ctx.call("GET", "/api/v2/edits?item_kind=track", ctx.admin_token, expect=(400,))
