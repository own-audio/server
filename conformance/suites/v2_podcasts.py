# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 podcast feeds and the directory (completion plan phase 2): follow, unfollow, refresh, auto-store, the back catalogue and single episodes kept on the server, unplayed counts, transcripts, and the directory.

Following and refreshing read a real feed: the suite serves one from this machine, which the
server reaches as `host.docker.internal` (a loopback `--base-url`, and a server started with
`SERVER__OUTBOUND__ALLOW_PRIVATE=true`; `conformance/compose.feeds.yml` adds the name on Linux).
Without those, the feed checks are skipped and the rest still runs. The directory's catalogue
routes need the metadata service (`modules.podcast_discovery`); where it is off they must answer
`501 feature_unavailable`, which is what is checked then. Search through Apple's public directory
(`modules.podcast_search` without the catalogue) may also answer `502` when Apple does not.
"""
from __future__ import annotations

import email.utils
import datetime
import http.server
import json
import threading
import urllib.parse
import uuid

from core import Ctx

NAME = "v2_podcasts"
PW = "V2PodcastsTest123!"
PROBLEM = "application/problem+json"
HOST = "host.docker.internal"


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


def _json(answer):
    try:
        return json.loads(answer[2]) if answer[2] else {}
    except ValueError:
        return {}


class Feeds:
    """Feeds the server fetches from this machine. Each feed is a list of episodes the suite can
    add to; one episode carries a WebVTT transcript with a speaker."""

    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx
        self.shows: dict[str, list[dict]] = {}
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                path = urllib.parse.urlsplit(self.path).path
                status, ctype, body = 200, "application/rss+xml", b""
                if path.startswith("/rss/") and path[5:] in outer.shows:
                    body = outer.render(path[5:]).encode()
                elif path.startswith("/audio/"):
                    ctype, body = "audio/mpeg", ctx.audio()
                elif path == "/t.vtt":
                    ctype = "text/vtt"
                    body = b"WEBVTT\n\n00:00:01.000 --> 00:00:03.500\n<v Anna Host>Welcome to the show.\n\n00:00:04.000 --> 00:00:06.000\nThanks for listening.\n"
                elif path == "/page":
                    ctype, body = "text/html", b"<html><body>not a feed</body></html>"
                else:
                    status, ctype, body = 404, "text/plain", b"no"
                self.send_response(status)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.server = http.server.ThreadingHTTPServer(("0.0.0.0", 0), Handler)
        self.port = self.server.server_address[1]
        self._stopped = False
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def url(self, path: str) -> str:
        return f"http://{HOST}:{self.port}{path}"

    def add_show(self, name: str, episodes: int) -> str:
        self.shows[name] = []
        for _ in range(episodes):
            self.add_episode(name)
        return self.url(f"/rss/{name}")

    def add_episode(self, name: str) -> dict:
        n = len(self.shows[name])
        ep = {"guid": f"{name}-ep{n}", "title": f"Episode {n} of {name}",
              "at": datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc) + datetime.timedelta(days=n)}
        self.shows[name].append(ep)
        return ep

    def render(self, name: str) -> str:
        items = []
        for i, ep in enumerate(self.shows[name]):
            transcript = f"<podcast:transcript url='{self.url('/t.vtt')}' type='text/vtt'/>" if i == 0 else ""
            items.append(
                f"<item><guid>{ep['guid']}</guid><title>{ep['title']}</title>"
                f"<pubDate>{email.utils.format_datetime(ep['at'])}</pubDate><itunes:duration>1:30</itunes:duration>"
                f"<enclosure url='{self.url('/audio/' + ep['guid'] + '.mp3')}' type='audio/mpeg' length='1'/>{transcript}</item>")
        return ("<?xml version='1.0'?><rss version='2.0' xmlns:itunes='http://www.itunes.com/dtds/podcast-1.0.dtd' "
                "xmlns:podcast='https://podcastindex.org/namespace/1.0'><channel>"
                f"<title>Show {name}</title><link>http://example.com/{name}</link><description>About {name}</description>"
                f"<language>en-US</language><itunes:author>Host of {name}</itunes:author>{''.join(items)}</channel></rss>")

    def stop(self) -> None:
        if not self._stopped:
            self._stopped = True
            self.server.shutdown()
            self.server.server_close()


class Changes:
    """The change feed from a starting point, read to its end each time."""

    def __init__(self, ctx: Ctx, token: str) -> None:
        self.ctx, self.token = ctx, token
        st, _h, raw = ctx.request("GET", "/api/v2/sync/changes", token)
        self.cursor = json.loads(raw)["cursor"] if st == 200 else ""

    def read(self) -> dict[tuple[str, str], dict]:
        seen: list[dict] = []
        for _ in range(50):
            st, _h, raw = self.ctx.request("GET", f"/api/v2/sync/changes?cursor={self.cursor}", self.token)
            if st != 200:
                break
            page = json.loads(raw)
            seen += page.get("changes", [])
            self.cursor = page.get("cursor", self.cursor)
            if not page.get("has_more"):
                break
        return {(c["kind"], c["id"]): c for c in seen}


def run(ctx: Ctx) -> None:
    check = ctx.check
    sfx = ctx.sfx
    owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    _mid, _me, member = ctx.make_member("member", PW, owner)
    nu_id, _ne, no_upload = ctx.make_member("noupload", PW, owner)
    ctx.call("PATCH", f"/api/v2/family/members/{nu_id}", owner, {"can_upload": False}, expect=(200, 204))

    directory(ctx, owner)
    uploaded_rules(ctx, owner, member)

    feeds = Feeds(ctx)
    ctx.on_cleanup("feed server", feeds.stop)
    if not ctx.is_local:
        ctx.skip("following a served feed", "needs a loopback --base-url: the server fetches the feed from this machine")
        unplayed_paging(ctx, owner)
        return
    main = feeds.add_show(f"main{sfx}", 3)
    probe = ctx.request("GET", "/api/v2/podcasts/directory/preview?" + urllib.parse.urlencode({"feed_url": main}), owner)
    if probe[0] == 400:
        ctx.skip("following a served feed", "the server refuses private addresses (SERVER__OUTBOUND__ALLOW_PRIVATE=true to test)")
        unplayed_paging(ctx, owner)
        return
    if probe[0] == 422:
        ctx.skip("following a served feed", f"the server cannot reach this machine as {HOST}")
        unplayed_paging(ctx, owner)
        return

    preview(ctx, feeds, owner, main)
    show = follow(ctx, feeds, owner, member, no_upload, nu_id, main)
    if not show:
        return
    refresh(ctx, feeds, owner, member, show)
    storing(ctx, owner, member, no_upload, nu_id, show)
    unplayed(ctx, owner, show)
    transcript(ctx, owner, member, show)
    unfollow(ctx, feeds, owner, member, show)
    unplayed_paging(ctx, owner)


def directory(ctx: Ctx, tok: str) -> None:
    """The catalogue's routes answer, or say the module is off; search answers or says why not."""
    check = ctx.check
    catalogue = ctx.feature("podcast_discovery")
    routes = [
        ("categories", "/api/v2/podcasts/directory/categories"),
        ("a category's shows", "/api/v2/podcasts/directory?category=news"),
        ("shows like one", "/api/v2/podcasts/directory/similar?" + urllib.parse.urlencode({"feed_url": "https://example.com/feed.xml"})),
    ]
    for label, path in routes:
        answer = ctx.request("GET", path, tok)
        if catalogue:
            check(f"directory {label}: a page", answer[0] == 200 and isinstance(_json(answer).get("items"), list), str(answer[0]))
        else:
            doc = _json(answer)
            check(f"directory {label} without a catalogue: 501 feature_unavailable",
                  _problem(answer, 501, "feature_unavailable") and doc.get("feature") == "podcast_discovery", str(answer[:1]))
    check("similar names a show: 400 invalid_query", _problem(ctx.request("GET", "/api/v2/podcasts/directory/similar", tok), 400, "invalid_query"))
    check("browse names a category", ctx.request("GET", "/api/v2/podcasts/directory?category=%20", tok)[0] == 400)
    check("search needs two characters: 400 invalid_query",
          _problem(ctx.request("GET", "/api/v2/podcasts/directory/search?q=a", tok), 400, "invalid_query"))
    check("a directory limit over 100: 400 invalid_limit",
          _problem(ctx.request("GET", "/api/v2/podcasts/directory/search?q=news&limit=101", tok), 400, "invalid_limit"))
    answer = ctx.request("GET", "/api/v2/podcasts/directory/search?q=history&limit=5", tok)
    if not ctx.feature("podcast_search"):
        check("search without a directory: 501 feature_unavailable", _problem(answer, 501, "feature_unavailable"))
    elif answer[0] == 502:
        check("search whose directory did not answer: 502 directory_unavailable", _problem(answer, 502, "directory_unavailable"))
    else:
        items = _json(answer).get("items", [])
        check("search: a page of shows, each with its feed", answer[0] == 200 and len(items) <= 5
              and all(i.get("feed_url") and i.get("title") and "podcast_id" in i for i in items), str(answer[0]))


def uploaded_rules(ctx: Ctx, owner: str, member: str) -> None:
    """What an uploaded show is not: followed, fetched, or unfollowed."""
    show = ctx.call("POST", "/api/v2/podcasts", owner, {"title": f"Made Here {ctx.sfx}", "visibility": "family"}, expect=(201,))
    check = ctx.check
    check("an uploaded show says so", show.get("source") == "upload" and show.get("auto_store") is False, str(show)[:160])
    check("refreshing an uploaded show: 409 uploaded_podcast",
          _problem(ctx.request("POST", f"/api/v2/podcasts/{show['id']}/refresh", owner), 409, "uploaded_podcast"))
    check("auto-store on an uploaded show: 409 uploaded_podcast",
          _problem(ctx.request("PATCH", f"/api/v2/podcasts/{show['id']}", owner, {"auto_store": True}), 409, "uploaded_podcast"))
    check("storing an uploaded show's back catalogue: 409 uploaded_podcast",
          _problem(ctx.request("POST", f"/api/v2/podcasts/{show['id']}/store", owner, {}), 409, "uploaded_podcast"))
    check("another member may not unfollow a show shared with them: 403",
          _problem(ctx.request("DELETE", f"/api/v2/podcasts/{show['id']}", member), 403))
    check("…nor change it: 403", _problem(ctx.request("PATCH", f"/api/v2/podcasts/{show['id']}", member, {"auto_store": True}), 403))
    credited = ctx.request("PUT", f"/api/v2/podcasts/{show['id']}/credits", owner,
                           {"credits": [{"name": f"Show Host {ctx.sfx}", "role": "host"}]})
    check("PUT /podcasts/{id}/credits: the show's hosts as contributors",
          credited[0] == 200 and [c.get("role") for c in _json(credited)] == ["host"] and _json(credited)[0].get("contributor_id"),
          f"{credited[0]} {credited[2][:200]!r}")
    check("…a role a show has not: 400 invalid_role", _problem(ctx.request(
        "PUT", f"/api/v2/podcasts/{show['id']}/credits", owner, {"credits": [{"name": "X", "role": "narrator"}]}), 400, "invalid_role"))
    check("…another member's show: 403", _problem(ctx.request("PUT", f"/api/v2/podcasts/{show['id']}/credits", member, {"credits": []}), 403))
    # Since revision 13 an uploaded show is deleted into the trash, not unfollowed (suite v2_folder).
    check("deleting an uploaded show: 204, into the trash (revision 13; 409 uploaded_podcast before)",
          ctx.request("DELETE", f"/api/v2/podcasts/{show['id']}", owner)[0] == 204)
    for body, code, label in [
        ({"feed_url": "not an address"}, "invalid_feed_url", "not an address"),
        ({"feed_url": "ftp://example.com/feed"}, "invalid_feed_url", "not http"),
        ({"feed_url": "https://www.youtube.com/@someone"}, "youtube_not_supported", "YouTube"),
        ({"feed_url": "https://example.com/feed.xml", "visibility": "everyone"}, "invalid_visibility", "a bad visibility"),
    ]:
        check(f"following {label}: 400 {code}", _problem(ctx.request("POST", "/api/v2/podcasts", owner, body), 400, code))
    check("an episode without a transcript: 404 no_transcript or not found",
          ctx.request("GET", f"/api/v2/episodes/{uuid.uuid4()}/transcript", owner)[0] == 404)


def preview(ctx: Ctx, feeds: Feeds, tok: str, url: str) -> None:
    check = ctx.check
    answer = ctx.request("GET", "/api/v2/podcasts/directory/preview?" + urllib.parse.urlencode({"feed_url": url, "limit": 2}), tok)
    doc = _json(answer)
    eps = doc.get("episodes", [])
    check("preview: the show as its feed says, not followed", answer[0] == 200 and doc.get("show", {}).get("title", "").startswith("Show main")
          and doc["show"].get("podcast_id") is None and doc["show"].get("author", "").startswith("Host of")
          and doc["show"].get("episode_count") == 3, str(doc)[:200])
    check("preview: the newest episodes first, `limit` of them, with their files",
          [e.get("title", "")[:9] for e in eps] == ["Episode 2", "Episode 1"] and eps[0].get("duration_ms") == 90_000
          and str(eps[0].get("audio_url", "")).endswith(".mp3"), str(eps)[:200])
    check("preview of something that is not a feed: 422 feed_unreadable",
          _problem(ctx.request("GET", "/api/v2/podcasts/directory/preview?" + urllib.parse.urlencode({"feed_url": feeds.url("/page")}), tok),
                   422, "feed_unreadable"))
    check("preview of an address that answers 404: 422 feed_unreadable",
          _problem(ctx.request("GET", "/api/v2/podcasts/directory/preview?" + urllib.parse.urlencode({"feed_url": feeds.url("/gone")}), tok),
                   422, "feed_unreadable"))


def follow(ctx: Ctx, feeds: Feeds, owner: str, member: str, no_upload: str, nu_id: str, url: str) -> dict | None:
    check = ctx.check
    changes = Changes(ctx, owner)
    theirs = Changes(ctx, member)
    key = f"follow-{ctx.sfx}"
    body = {"feed_url": url, "visibility": "family"}
    first = ctx.request("POST", "/api/v2/podcasts", owner, body, headers={"Idempotency-Key": key})
    show = _json(first)
    if not check("follow: 201 and the show, from its feed", first[0] == 201 and show.get("source") == "rss"
                 and show.get("feed_url") == url and show.get("title", "").startswith("Show main")
                 and show.get("visibility") == "family", str(first[:1]) + str(show)[:200]):
        return None
    again = ctx.request("POST", "/api/v2/podcasts", owner, body, headers={"Idempotency-Key": key})
    check("the same Idempotency-Key answers the same, once", again[0] == 201 and again[2] == first[2]
          and again[1].get("idempotent-replayed") == "true")
    check("the same key with another body: 422 idempotency_key_reused",
          _problem(ctx.request("POST", "/api/v2/podcasts", owner, {"feed_url": url}, headers={"Idempotency-Key": key}), 422, "idempotency_key_reused"))
    second = ctx.request("POST", "/api/v2/podcasts", owner, {"feed_url": url})
    check("following it again: 200 and the same show", second[0] == 200 and _json(second).get("id") == show["id"])
    shared = ctx.request("POST", "/api/v2/podcasts", member, {"feed_url": url})
    check("a member who sees it shared, following it: 200 and the same show", shared[0] == 200 and _json(shared).get("id") == show["id"])
    listed = ctx.call("GET", f"/api/v2/podcasts?q={urllib.parse.quote('Show main' + ctx.sfx)}", owner)["items"]
    check("…one show in the library, not several", [p["id"] for p in listed] == [show["id"]], str(listed)[:200])
    eps = ctx.call("GET", f"/api/v2/podcasts/{show['id']}/episodes", owner)["items"]
    check("its episodes are in at once, newest first", [e["guid"] for e in eps] == [f"main{ctx.sfx}-ep{i}" for i in (2, 1, 0)]
          and all(e["duration_ms"] == 90_000 and e["stored"] is False for e in eps), str(eps)[:200])
    seen = changes.read()
    check("the follow is in the change feed: the show and its episodes",
          seen.get(("podcast", show["id"]), {}).get("op") == "upsert" and all(("episode", e["id"]) in seen for e in eps), str(list(seen))[:200])
    check("…and in a household member's, for a shared show", theirs.read().get(("podcast", show["id"]), {}).get("op") == "upsert")
    preview = _json(ctx.request("GET", "/api/v2/podcasts/directory/preview?" + urllib.parse.urlencode({"feed_url": url}), owner))
    check("preview now names the show here", preview.get("show", {}).get("podcast_id") == show["id"])

    own = feeds.add_show(f"own{ctx.sfx}", 1)
    theirs_follow = ctx.request("POST", "/api/v2/podcasts", no_upload, {"feed_url": own})
    check("a member who may not upload may still follow a feed", theirs_follow[0] == 201 and _json(theirs_follow).get("visibility") == "private")
    check("…but not make an uploaded show: 403", _problem(ctx.request("POST", "/api/v2/podcasts", no_upload, {"title": f"Nope {ctx.sfx}"}), 403))
    private_id = _json(theirs_follow).get("id")
    if private_id:
        for method, path in [("POST", f"/api/v2/podcasts/{private_id}/refresh"), ("DELETE", f"/api/v2/podcasts/{private_id}"),
                             ("PATCH", f"/api/v2/podcasts/{private_id}"), ("POST", f"/api/v2/podcasts/{private_id}/store")]:
            check(f"someone else's private show: {method} {path.split(private_id)[1] or '/'} is 404",
                  _problem(ctx.request(method, path, member, {} if method != "DELETE" else None), 404, "not_found"))
        ctx.call("DELETE", f"/api/v2/podcasts/{private_id}", no_upload, expect=(204,))
    return show


def refresh(ctx: Ctx, feeds: Feeds, owner: str, member: str, show: dict) -> None:
    check = ctx.check
    changes = Changes(ctx, owner)
    new = feeds.add_episode(f"main{ctx.sfx}")
    answer = ctx.request("POST", f"/api/v2/podcasts/{show['id']}/refresh", member)
    check("refresh by anyone who sees the show: 202 queued", answer[0] == 202 and _json(answer).get("queued") == 1, str(answer[:1]))
    arrived: dict = {}

    def there() -> bool:
        nonlocal arrived
        arrived.update(changes.read())
        return any(k == "episode" and c.get("record", {}).get("guid") == new["guid"] for (k, _i), c in arrived.items())
    check("…the new episode arrives through the change feed", ctx.wait_until(there, timeout=60, every=1))
    eps = ctx.call("GET", f"/api/v2/podcasts/{show['id']}/episodes", owner)["items"]
    check("…and is listed first", eps and eps[0]["guid"] == new["guid"] and len(eps) == 4, str([e["guid"] for e in eps]))
    refreshed = ctx.call("GET", f"/api/v2/podcasts/{show['id']}", owner)
    check("…the show says when it was read", bool(refreshed.get("last_refreshed_at")))
    every = ctx.request("POST", "/api/v2/podcasts/refresh", owner)
    check("refresh every show: 202 with how many", every[0] == 202 and _json(every).get("queued", 0) >= 1, str(every[:1]))


def storing(ctx: Ctx, owner: str, member: str, no_upload: str, nu_id: str, show: dict) -> None:
    check = ctx.check
    sid = show["id"]
    check("auto-store by a member who does not own the show: 403",
          _problem(ctx.request("PATCH", f"/api/v2/podcasts/{sid}", member, {"auto_store": True}), 403))
    changes = Changes(ctx, owner)
    on = ctx.request("PATCH", f"/api/v2/podcasts/{sid}", owner, {"auto_store": True})
    check("auto-store on: the show says so", on[0] == 200 and _json(on).get("auto_store") is True, str(on[:1]))
    check("…and the change feed carries it", changes.read().get(("podcast", sid), {}).get("record", {}).get("auto_store") is True)
    off = ctx.request("PATCH", f"/api/v2/podcasts/{sid}", owner, {"auto_store": False})
    check("auto-store off again", off[0] == 200 and _json(off).get("auto_store") is False)

    check("the back catalogue by another member: 403", _problem(ctx.request("POST", f"/api/v2/podcasts/{sid}/store", member, {"preview": True}), 403))
    check("latest below 1: 400 invalid_latest", _problem(ctx.request("POST", f"/api/v2/podcasts/{sid}/store", owner, {"latest": 0}), 400, "invalid_latest"))
    pv = ctx.request("POST", f"/api/v2/podcasts/{sid}/store", owner, {"preview": True})
    check("a preview counts the episodes not stored, queues nothing", pv[0] == 200 and _json(pv).get("episodes") == 4
          and _json(pv).get("estimated_bytes", 0) > 0, str(pv[:1]) + str(_json(pv)))
    one = ctx.request("POST", f"/api/v2/podcasts/{sid}/store", owner, {"latest": 1})
    check("the newest one: 202, one episode queued", one[0] == 202 and _json(one).get("episodes") == 1, str(_json(one)))

    def stored() -> list[str]:
        return [e["id"] for e in ctx.call("GET", f"/api/v2/podcasts/{sid}/episodes", owner)["items"] if e["stored"]]
    check("…stored by the server", ctx.wait_until(lambda: len(stored()) == 1, timeout=60, every=1), str(stored()))
    eps = ctx.call("GET", f"/api/v2/podcasts/{sid}/episodes", owner)["items"]
    oldest = eps[-1]

    check("keeping an episode by a member who may not add to the library: 403",
          _problem(ctx.request("PUT", f"/api/v2/episodes/{oldest['id']}/stored", no_upload), 403))
    changes = Changes(ctx, owner)
    put = ctx.request("PUT", f"/api/v2/episodes/{oldest['id']}/stored", member)
    check("keep one episode: 202 and the episode", put[0] == 202 and _json(put).get("id") == oldest["id"], str(put[:1]))
    seen: dict = {}

    def arrived() -> bool:
        seen.update(changes.read())
        return seen.get(("episode", oldest["id"]), {}).get("record", {}).get("stored") is True
    check("…stored, and the change feed says so", ctx.wait_until(arrived, timeout=60, every=1))
    again = ctx.request("PUT", f"/api/v2/episodes/{oldest['id']}/stored", member)
    check("keeping it again: 200, stored already", again[0] == 200 and _json(again).get("stored") is True)
    check("dropping the copy is the owner's: 403 to another member",
          _problem(ctx.request("DELETE", f"/api/v2/episodes/{oldest['id']}/stored", member), 403))
    dropped = ctx.request("DELETE", f"/api/v2/episodes/{oldest['id']}/stored", owner)
    check("drop the server's copy: the episode stays, not stored", dropped[0] == 200 and _json(dropped).get("stored") is False)
    check("…and the change feed says so", changes.read().get(("episode", oldest["id"]), {}).get("record", {}).get("stored") is False)
    listed = [e["id"] for e in ctx.call("GET", f"/api/v2/podcasts/{sid}/episodes", owner)["items"]]
    check("…still listed", oldest["id"] in listed)
    not_stored = ctx.request("DELETE", f"/api/v2/episodes/{oldest['id']}/stored", owner)
    check("dropping a copy that is not there answers the episode", not_stored[0] == 200 and _json(not_stored).get("stored") is False)


def unplayed(ctx: Ctx, tok: str, show: dict) -> None:
    check = ctx.check

    def count() -> int | None:
        items, cursor = [], None
        for _ in range(100):
            page = ctx.call("GET", "/api/v2/me/podcasts/unplayed?limit=50" + (f"&cursor={cursor}" if cursor else ""), tok)
            items += page["items"]
            cursor = page.get("next")
            if not cursor:
                break
        return next((i["unplayed"] for i in items if i["podcast_id"] == show["id"]), None)
    before = count()
    check("unplayed: every episode of a new show", before == 4, str(before))
    eps = ctx.call("GET", f"/api/v2/podcasts/{show['id']}/episodes", tok)["items"]
    ctx.call("PUT", f"/api/v2/me/progress/episode/{eps[0]['id']}", tok, {"position_ms": 90_000, "finished": True}, expect=(200, 204))
    ctx.call("PUT", f"/api/v2/me/progress/episode/{eps[1]['id']}", tok, {"position_ms": 1_000}, expect=(200, 204))
    check("…one fewer after one is finished (one begun still counts)", count() == 3)


def transcript(ctx: Ctx, owner: str, member: str, show: dict) -> None:
    check = ctx.check
    eps = ctx.call("GET", f"/api/v2/podcasts/{show['id']}/episodes", owner)["items"]
    first = next(e for e in eps if e["guid"].endswith("-ep0"))
    answer = ctx.request("GET", f"/api/v2/episodes/{first['id']}/transcript", member)
    doc = _json(answer)
    segs = doc.get("segments", [])
    check("the publisher's transcript as timed lines, with the speaker", answer[0] == 200 and doc.get("format") == "segments"
          and len(segs) == 2 and segs[0] == {"start_ms": 1000, "end_ms": 3500, "speaker": "Anna Host", "text": "Welcome to the show."}
          and "speaker" not in segs[1], str(doc)[:300])
    other = next(e for e in eps if not e["guid"].endswith("-ep0"))
    check("an episode without one: 404 no_transcript",
          _problem(ctx.request("GET", f"/api/v2/episodes/{other['id']}/transcript", owner), 404, "no_transcript"))


def unfollow(ctx: Ctx, feeds: Feeds, owner: str, member: str, show: dict) -> None:
    check = ctx.check
    eps = ctx.call("GET", f"/api/v2/podcasts/{show['id']}/episodes", owner)["items"]
    check("unfollowing someone else's shared show: 403", _problem(ctx.request("DELETE", f"/api/v2/podcasts/{show['id']}", member), 403))
    changes = Changes(ctx, owner)
    theirs = Changes(ctx, member)
    ctx.call("DELETE", f"/api/v2/podcasts/{show['id']}", owner, expect=(204,))
    check("unfollowed: gone", _problem(ctx.request("GET", f"/api/v2/podcasts/{show['id']}", owner), 404, "not_found"))
    seen = changes.read()
    check("the change feed sends the show's removal", seen.get(("podcast", show["id"]), {}).get("op") == "delete", str(list(seen))[:200])
    check("…and each episode's", all(seen.get(("episode", e["id"]), {}).get("op") == "delete" for e in eps),
          str([(k, c.get("op")) for k, c in seen.items()])[:300])
    check("…to the household too", theirs.read().get(("podcast", show["id"]), {}).get("op") == "delete")
    check("unfollowing twice: 404", _problem(ctx.request("DELETE", f"/api/v2/podcasts/{show['id']}", owner), 404))


def unplayed_paging(ctx: Ctx, owner: str) -> None:
    """The unplayed list, walked a page at a time against one long page."""
    for i in range(3):
        ctx.call("POST", "/api/v2/podcasts", owner, {"title": f"Unplayed Show {ctx.sfx} {i}", "guid": f"unplayed-{ctx.sfx}-{i}"}, expect=(200, 201))

    def walk(limit: int):
        items, cursor, pages = [], None, 0
        while True:
            page = ctx.call("GET", f"/api/v2/me/podcasts/unplayed?limit={limit}" + (f"&cursor={cursor}" if cursor else ""), owner)
            items += [i["podcast_id"] for i in page["items"]]
            cursor, pages = page.get("next"), pages + 1
            if not cursor or pages > 500:
                return items, pages
    full, _ = walk(500)
    small, pages = walk(1)
    ctx.check("unplayed pages without loss or repeats", small == full and len(set(small)) == len(small) and len(full) >= 3 and pages > 1,
              f"{len(small)} in {pages} pages against {len(full)}")
    ctx.check("…every show once, an empty one at zero",
              all(isinstance(i, str) for i in full) and full == sorted(full))
