# SPDX-License-Identifier: AGPL-3.0-or-later
"""gPodder sync API: basic auth with the app password, devices, subscriptions both ways, episode actions and progress.

The API podcast apps (AntennaPod, Kasts, gPodder, own.audio Podcasts) sync
with, at /api/2. Serves a small RSS feed from this machine, which the server
reaches as host.docker.internal, so a show subscribed through the API is
really fetched.
"""
from __future__ import annotations

import base64
import datetime
import email.utils
import http.server
import threading
import time

from core import Ctx

NAME = "gpodder"
REQUIRES = {"local"}
PW = "GpodderTest123!"


class _Feeds:
    """Two small RSS feeds, /a and /b, with two episodes each."""

    def __init__(self, ctx: Ctx) -> None:
        sfx = ctx.sfx
        outer = self
        now = datetime.datetime.now(datetime.timezone.utc)

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                name = self.path.strip("/").split("?")[0]
                if name in ("a", "b"):
                    items = "".join(
                        f"<item><guid>{name}{n}-{sfx}</guid><title>Episode {n}</title>"
                        f"<pubDate>{email.utils.format_datetime(now - datetime.timedelta(days=n))}</pubDate>"
                        f"<itunes:duration>600</itunes:duration>"
                        f"<enclosure url='{outer.base}/{name}{n}.mp3' type='audio/mpeg' length='1'/></item>"
                        for n in (1, 2)
                    )
                    body = (
                        "<?xml version='1.0'?><rss version='2.0' xmlns:itunes='http://www.itunes.com/dtds/podcast-1.0.dtd'>"
                        f"<channel><title>Show {name.upper()} {sfx}</title><link>http://example.com</link>"
                        f"<description>A test show</description>{items}</channel></rss>"
                    ).encode()
                    ctype = "application/rss+xml"
                else:
                    body, ctype = ctx.audio(), "audio/mpeg"
                self.send_response(200)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.server = http.server.ThreadingHTTPServer(("0.0.0.0", 0), Handler)
        self.base = f"http://host.docker.internal:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()


def run(ctx: Ctx) -> None:
    ctx.require_feature("gpodder")
    check, call = ctx.check, ctx.call
    uid, email_addr, tok = ctx.make_user("listener", PW)
    _, other_email, _ = ctx.make_user("other", PW)
    key = call("GET", "/api/v1/users/me/subsonic-key", tok)["api_key"]

    def basic(user: str, secret: str) -> dict[str, str]:
        return {"Authorization": "Basic " + base64.b64encode(f"{user}:{secret}".encode()).decode()}

    auth = basic(email_addr, key)

    def g(method: str, path: str, body=None, headers=None, expect=(200,)):
        return call(method, "/api/2" + path, body=body, headers=headers or auth, expect=expect)

    feeds = _Feeds(ctx)
    ctx.on_cleanup("rss feeds", feeds.stop)
    feed_a, feed_b = f"{feeds.base}/a", f"{feeds.base}/b"

    # ── Signing in ────────────────────────────────────────────────────────
    ctx.log("\n[sign-in]")
    status, headers, _ = ctx.request("POST", f"/api/2/auth/{email_addr}/login.json")
    check("no credentials: 401 with a basic challenge", status == 401 and "basic" in headers.get("www-authenticate", "").lower())
    check("the account password is not the app password",
          ctx.request("POST", f"/api/2/auth/{email_addr}/login.json", headers=basic(email_addr, PW))[0] == 401)
    check("a wrong app password is refused",
          ctx.request("POST", f"/api/2/auth/{email_addr}/login.json", headers=basic(email_addr, key + "x"))[0] == 401)
    g("POST", f"/auth/{email_addr}/login.json")
    check("the app password signs in, the email in any case",
          ctx.request("POST", f"/api/2/auth/{email_addr.upper()}/login.json", headers=basic(email_addr.upper(), key))[0] == 200)
    check("another account's path is forbidden",
          ctx.request("GET", f"/api/2/devices/{other_email}.json", headers=auth)[0] == 403)

    # ── Devices ───────────────────────────────────────────────────────────
    ctx.log("\n[devices]")
    g("POST", f"/devices/{email_addr}/phone-{ctx.sfx}.json", {"caption": "Test phone", "type": "mobile"})
    devices = g("GET", f"/devices/{email_addr}.json")
    mine = [d for d in devices if d["id"] == f"phone-{ctx.sfx}"]
    check("a registered device is listed with its caption and type",
          bool(mine) and mine[0]["caption"] == "Test phone" and mine[0]["type"] == "mobile")
    check("a device id with a space is refused",
          ctx.request("POST", f"/api/2/devices/{email_addr}/bad%20id.json", body={}, headers=auth)[0] == 400)
    device = f"/subscriptions/{email_addr}/phone-{ctx.sfx}.json"

    # ── Subscriptions: from the app to the server ─────────────────────────
    ctx.log("\n[subscriptions up]")
    first = g("GET", device + "?since=0")
    check("a new listener has no subscriptions", first["add"] == [] and first["remove"] == [])
    answer = g("POST", device, {"add": [f"  {feed_a}  "], "remove": []})
    check("a URL sent with spaces is cleaned and reported", [f"  {feed_a}  ", feed_a] in answer["update_urls"])
    other_device = f"/subscriptions/{email_addr}/tablet-{ctx.sfx}.json"
    check("the app that added a show is not told about it again",
          feed_a not in g("GET", f"{device}?since={first['timestamp']}")["add"])
    check("the listener's other apps are", feed_a in g("GET", f"{other_device}?since={first['timestamp']}")["add"])
    check("the same feed added and removed at once is refused",
          ctx.request("POST", "/api/2" + device, body={"add": [feed_b], "remove": [feed_b]}, headers=auth)[0] == 400)

    def podcast(url: str):
        return next((p for p in call("GET", "/api/v1/podcasts", tok) if p["feed_url"] == url), None)

    check("the show added through the API is one of the listener's podcasts", podcast(feed_a) is not None)
    check("the worker reads its title and episodes",
          ctx.wait_until(lambda: (p := podcast(feed_a)) is not None and p["title"] == f"Show A {ctx.sfx}"
                         and len(call("GET", f"/api/v1/podcasts/{p['id']}/episodes", tok)) == 2, timeout=60))

    # ── Subscriptions: from the server to the app ─────────────────────────
    ctx.log("\n[subscriptions down]")
    time.sleep(6)  # past the read-again margin, so only new changes come back
    since = g("GET", device + "?since=0")["timestamp"]
    show_b = call("POST", "/api/v1/podcasts/subscribe", tok, {"feed_url": feed_b})
    down = g("GET", f"{device}?since={since}")
    check("a show followed in the app's own API reaches gPodder apps", feed_b in down["add"])
    check("the answer carries a timestamp to ask from next time", isinstance(down["timestamp"], int) and down["timestamp"] >= since)
    everything = g("GET", device + "?since=0")
    check("since=0 lists every subscription", set(everything["add"]) >= {feed_a, feed_b})

    # ── Episode actions and progress ──────────────────────────────────────
    ctx.log("\n[episode actions]")
    episodes_b = call("GET", f"/api/v1/podcasts/{show_b['id']}/episodes", tok)
    ep = next(e for e in episodes_b if e["audio_url"].endswith("/b1.mp3"))
    play_at = (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=5)).strftime("%Y-%m-%dT%H:%M:%S")
    up = g("POST", f"/episodes/{email_addr}.json", [{
        "podcast": feed_b, "episode": ep["audio_url"], "action": "play", "device": f"phone-{ctx.sfx}",
        "timestamp": play_at, "started": 0, "position": 120, "total": 600,
    }])
    check("an upload answers with a timestamp", isinstance(up["timestamp"], int))

    def progress_of(episode_id: str):
        e = next(x for x in call("GET", f"/api/v1/podcasts/{show_b['id']}/episodes", tok) if x["id"] == episode_id)
        return e["progress_secs"], e["completed"]

    check("a play action becomes the listener's position", progress_of(ep["id"]) == (120.0, False))
    older = (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(hours=2)).strftime("%Y-%m-%dT%H:%M:%S")
    g("POST", f"/episodes/{email_addr}.json", [{
        "podcast": feed_b, "episode": ep["audio_url"], "action": "play", "timestamp": older, "position": 10, "total": 600,
    }])
    check("an older action does not move it back", progress_of(ep["id"]) == (120.0, False))
    g("POST", f"/episodes/{email_addr}.json", [{
        "podcast": feed_b, "episode": ep["audio_url"], "action": "play", "position": 590, "total": 600,
    }])
    check("near the end counts as heard", progress_of(ep["id"]) == (590.0, True))
    check("an unknown action is refused",
          ctx.request("POST", f"/api/2/episodes/{email_addr}.json", body=[{"podcast": feed_b, "episode": ep["audio_url"], "action": "flattr"}], headers=auth)[0] == 400)

    # Answers repeat the last few seconds before `since`; wait those out so the
    # actions above are not in the window.
    time.sleep(6)
    since = g("GET", f"/episodes/{email_addr}.json?since=0")["timestamp"]
    ep2 = next(e for e in episodes_b if e["audio_url"].endswith("/b2.mp3"))
    call("PUT", f"/api/v1/playback/episodes/{ep2['id']}/progress", tok, {"position_secs": 300, "completed": False})
    actions = g("GET", f"/episodes/{email_addr}.json?since={since}")["actions"]
    mine = [a for a in actions if a["episode"] == ep2["audio_url"]]
    check("a position saved in the own.audio apps reaches gPodder apps as a play action",
          len(mine) == 1 and mine[0]["action"] == "play" and mine[0]["position"] == 300 and mine[0]["total"] == 600
          and mine[0]["podcast"] == feed_b and "device" not in mine[0])
    repeated = [a for a in actions if a["episode"] == ep["audio_url"]]
    check("an action the API applied is not logged a second time", not repeated, str(repeated))
    full = g("GET", f"/episodes/{email_addr}.json?since=0")["actions"]
    aggregated = g("GET", f"/episodes/{email_addr}.json?since=0&aggregated=true")["actions"]
    check("aggregated keeps the latest action per episode",
          len([a for a in full if a["episode"] == ep["audio_url"]]) == 3
          and [a["position"] for a in aggregated if a["episode"] == ep["audio_url"]] == [590])
    check("the device filter keeps one device's actions",
          all(a.get("device") == f"phone-{ctx.sfx}" for a in g("GET", f"/episodes/{email_addr}.json?since=0&device=phone-{ctx.sfx}")["actions"]))
    check("timestamps are the API's own form", all(len(a["timestamp"]) == 19 and a["timestamp"][10] == "T" for a in full))

    # ── Removing, both ways ───────────────────────────────────────────────
    ctx.log("\n[removing]")
    time.sleep(6)
    since = g("GET", device + "?since=0")["timestamp"]
    g("POST", device, {"add": [], "remove": [feed_a]})
    check("a show removed through the API leaves the listener's podcasts", podcast(feed_a) is None)
    call("DELETE", f"/api/v1/podcasts/{show_b['id']}", tok, expect=(204,))
    down = g("GET", f"{device}?since={since}")
    check("a removal in the console reaches the app; its own is not repeated to it",
          down["remove"] == [feed_b] and down["add"] == [], str(down))
    down = g("GET", f"{other_device}?since={since}")
    check("the listener's other apps get both removals", set(down["remove"]) >= {feed_a, feed_b} and down["add"] == [], str(down))
