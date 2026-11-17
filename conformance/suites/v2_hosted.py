# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 on the hosted edition (completion plan phase 13): the credit, the administrator's grant, narration and translation quotes, translations as a kind, questions paged.

Each part runs only where the server offers its module (`ctx.feature("billing" | "narration" |
"translation" | "summaries")`, the README's rule for checks that hold on one edition); on a server
without billing the administrator's credit grant is `501 feature_unavailable`, named. Paid work is
asked for no further than a quote or an estimate.

- **Billing.** The household's credit in minor units with its currency beside the millionths;
  `POST /admin/families/{id}/credit` only for the server's administrator, only with an
  `Idempotency-Key` (a retry grants once, another body under the same key is refused), in whole
  minor units of the ledger's currency; the grant in `GET /admin/families/{id}` and the server's
  total in `GET /admin/stats`.
- **Narration.** Languages, voices, the cover brief, a quote and an estimate; an unknown job is a
  `404` problem.
- **Translations** (with `--compose-dir`: a finished translation is made through SQL, since making
  one is paid work). Played through `POST /playback-info` with kind `translation`; its progress
  through `/me/progress/translation/{id}`, the later position winning, and through the shipped
  apps' `PUT /translation/episodes/{id}/progress`, which writes the same position; both in the
  change feed (`translation`, `progress.translation`); the lists in milliseconds and paged when
  asked; a `translation` event on `/events` when one starts; another household sees none of it.
- **Questions** (with `--compose-dir`): kept questions seeded through SQL, paged without loss.
"""
from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request
import uuid

from core import Ctx
from suites.v2_books import _problem

NAME = "v2_hosted"
PW = "V2HostedTest12345!"


def _json(answer) -> dict:
    try:
        return json.loads(answer[2])
    except ValueError:
        return {}


def run(ctx: Ctx) -> None:
    if ctx.feature("billing"):
        billing(ctx)
    else:
        got = ctx.request("POST", f"/api/v2/admin/families/{uuid.uuid4()}/credit", ctx.admin_token,
                          {"amount_minor": 100, "currency": "USD"}, headers={"Idempotency-Key": f"off-{ctx.sfx}"})
        ctx.check("no billing: the administrator's credit grant is 501 feature_unavailable, named",
                  _problem(got, 501, "feature_unavailable") and _json(got).get("feature") == "billing")
    if ctx.feature("narration"):
        narration(ctx)
    else:
        ctx.skip("narration", "the server does not offer narration")
    if ctx.feature("translation") or ctx.feature("summaries"):
        spoken(ctx)
    else:
        ctx.skip("translations and questions", "the server offers neither")


# ── Billing ───────────────────────────────────────────────────────────────────

def _grant(ctx: Ctx, fid: str, token: str | None, body: dict, key: str | None):
    headers = {"Idempotency-Key": key} if key else {}
    return ctx.request("POST", f"/api/v2/admin/families/{fid}/credit", token, body, headers=headers)


def billing(ctx: Ctx) -> None:
    uid, _e, owner = ctx.make_family_admin("payer", PW)
    fid = ctx.call("GET", "/api/v2/me", owner)["family"]["id"]
    before = ctx.call("GET", "/api/v2/family/billing", owner)
    micro = before.get("balance_micro")
    ctx.check("the credit in minor units beside the millionths, with its currency",
              before.get("currency") == "USD" and isinstance(micro, int) and before.get("balance_minor") == int(micro / 10_000)
              and before.get("available_minor") == int(before["available_micro"] / 10_000)
              and all(e.get("amount_minor") == int(e["amount_micro"] / 10_000) for e in before.get("entries", [])),
              str({k: before.get(k) for k in ("currency", "balance_minor", "balance_micro")}))
    v1 = ctx.call("GET", "/api/v1/family/billing", owner)
    ctx.check("…and v1 still answers its millionths", v1.get("balance_micro") == micro)

    body = {"amount_minor": 250, "currency": "USD", "note": f"conformance {ctx.sfx}"}
    ctx.check("the credit grant is refused to a household admin (403)", _problem(_grant(ctx, fid, owner, body, f"k-{ctx.sfx}"), 403))
    ctx.check("…and needs an Idempotency-Key (400 idempotency_key_required)",
              _problem(_grant(ctx, fid, ctx.admin_token, body, None), 400, "idempotency_key_required"))
    ctx.check("…in the ledger's currency (400 unsupported_currency)",
              _problem(_grant(ctx, fid, ctx.admin_token, dict(body, currency="EUR"), f"eur-{ctx.sfx}"), 400, "unsupported_currency"))
    ctx.check("…of something (400 invalid_amount)",
              _problem(_grant(ctx, fid, ctx.admin_token, dict(body, amount_minor=0), f"zero-{ctx.sfx}"), 400, "invalid_amount"))
    ctx.check("…to a household that exists (404)",
              _problem(_grant(ctx, str(uuid.uuid4()), ctx.admin_token, body, f"none-{ctx.sfx}"), 404))

    key = f"grant-{ctx.sfx}"
    first = _grant(ctx, fid, ctx.admin_token, body, key)
    grant = _json(first)
    entry = grant.get("entry") or {}
    ctx.check("the administrator grants credit: 201, the entry and the balance after it",
              first[0] == 201 and entry.get("kind") == "adjustment" and entry.get("amount_minor") == 250
              and entry.get("amount_micro") == 2_500_000 and grant.get("currency") == "USD"
              and grant.get("balance_micro") == micro + 2_500_000, str(grant)[:300])
    again = _grant(ctx, fid, ctx.admin_token, body, key)
    after = ctx.call("GET", "/api/v2/family/billing", owner)
    ctx.check("…a retry with the same key answers the same and grants once",
              again[0] == 201 and (_json(again).get("entry") or {}).get("id") == entry.get("id")
              and after.get("balance_micro") == micro + 2_500_000, f"{again[0]} {after.get('balance_micro')}")
    ctx.check("…another body under the same key is refused (422 idempotency_key_reused)",
              _problem(_grant(ctx, fid, ctx.admin_token, dict(body, amount_minor=999), key), 422, "idempotency_key_reused"))
    ctx.on_cleanup("take the granted credit back", lambda: _grant(ctx, fid, ctx.admin_token, dict(body, amount_minor=-250), f"back-{ctx.sfx}"))

    ctx.call("PUT", "/api/v2/family/billing/alerts", owner, {"min_balance_minor": 150, "min_days_remaining": None}, expect=(204,))
    alerts = ctx.call("GET", "/api/v2/family/billing/alerts", owner)
    ctx.check("the household's alerts: a threshold given in whole cents is kept in both units",
              alerts.get("min_balance_minor") == 150 and alerts.get("min_balance_micro") == 1_500_000, str(alerts))

    detail = ctx.call("GET", f"/api/v2/admin/families/{fid}", ctx.admin_token)
    credit = detail.get("credit") or {}
    ctx.check("the household's credit for the administrator: balance and the grant among its entries",
              credit.get("balance_micro") == micro + 2_500_000 and credit.get("balance_minor") == int((micro + 2_500_000) / 10_000)
              and credit.get("currency") == "USD"
              and any(e.get("id") == entry.get("id") and body["note"] in (e.get("note") or "") for e in credit.get("entries", [])),
              str(credit)[:300])
    stats = ctx.call("GET", "/api/v2/admin/stats", ctx.admin_token)
    total = stats.get("credit") or {}
    ctx.check("the server's credit in its figures", total.get("currency") == "USD" and isinstance(total.get("balance_micro"), int)
              and total.get("balance_minor") == int(total["balance_micro"] / 10_000), str(total))


# ── Narration ─────────────────────────────────────────────────────────────────

def _multipart(fields: dict[str, tuple[str, bytes, str]]) -> tuple[bytes, str]:
    boundary = uuid.uuid4().hex
    out = b""
    for name, (filename, content, content_type) in fields.items():
        out += (f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
                f"Content-Type: {content_type}\r\n\r\n").encode() + content + b"\r\n"
    return out + f"--{boundary}--\r\n".encode(), f"multipart/form-data; boundary={boundary}"


def narration(ctx: Ctx) -> None:
    _uid, _e, tok = ctx.make_family_admin("narrator", PW)
    languages = ctx.call("GET", "/api/v2/narration/languages", tok)
    voices = ctx.call("GET", "/api/v2/narration/voices", tok)
    prompt = ctx.call("GET", "/api/v2/narration/cover-prompt", tok)
    ctx.check("narration: languages, voices and the cover brief",
              isinstance(languages, list) and isinstance(voices, list) and voices and bool(prompt.get("prompt")))
    if not voices:
        return
    quote = ctx.call("POST", "/api/v2/narration/quote", tok, {"char_count": 20_000, "source_language": "en", "target_language": "cs",
                                                              "voice_profile_id": voices[0]["id"]})
    ctx.check("…a quote in whole cents with its currency",
              isinstance(quote.get("quoted_price_cents"), int) and quote["quoted_price_cents"] > 0 and quote.get("currency") == "USD", str(quote))
    data, content_type = _multipart({"file": ("story.txt", ("Once upon a time. " * 200).encode(), "text/plain")})
    got = ctx.request("POST", "/api/v2/narration/estimate", tok, data, headers={"Content-Type": content_type})
    ctx.check("…an estimate of a text's length", got[0] == 200 and _json(got).get("estimated_char_count", 0) > 0, str(got[:2]))
    ctx.check("…an unknown job is a 404 problem", _problem(ctx.request("GET", f"/api/v2/narration/jobs/{uuid.uuid4()}", tok), 404))


# ── Translations and questions ────────────────────────────────────────────────

def _upload_episode(ctx: Ctx, tok: str) -> tuple[str, str]:
    show = ctx.call("POST", "/api/v2/podcasts", tok, {"title": f"Hosted Show {ctx.sfx}", "visibility": "private"}, expect=(200, 201))
    audio = ctx.tone()
    made = ctx.call("POST", "/api/v2/uploads", tok, {"kind": "episode", "filename": "e.wav", "content_type": "audio/wav",
                                                     "size_bytes": len(audio)}, expect=(201,))
    ctx.put_bytes(made["url"], audio, made["headers"]["Content-Type"])
    ep = ctx.call("POST", f"/api/v2/podcasts/{show['id']}/episodes", tok, {
        "upload_id": made["upload_id"], "guid": f"hosted-{ctx.sfx}", "title": f"Hosted Episode {ctx.sfx}"}, expect=(200, 201))
    return show["id"], ep["id"]


def _translation_row(ctx: Ctx, tid: str, uid: str, fid: str, eid: str, status: str) -> None:
    ctx.sql(f"""INSERT INTO podcast_episode_translations
                  (id, episode_id, user_id, family_id, voice_profile_id, source_language, target_language, status,
                   char_count, quoted_price_cents, audio_object_id, duration_secs)
                SELECT '{tid}', e.id, '{uid}', '{fid}', (SELECT id FROM voice_profiles ORDER BY id LIMIT 1), 'en',
                       '{'cs' if status == 'complete' else 'de'}', '{status}', 100, 1, e.audio_object_id, 4
                  FROM podcast_episodes e WHERE e.id = '{eid}'""")
    ctx.on_cleanup(f"drop translation {tid}", lambda: ctx.sql(f"DELETE FROM podcast_episode_translations WHERE id = '{tid}'"))


def _event_after(ctx: Ctx, token: str, name: str, action) -> bool:
    req = urllib.request.Request(ctx.url("/api/v2/events"), headers={"Authorization": f"Bearer {token}", "Accept": "text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            action()
            deadline = time.time() + 20
            for raw in r:
                if raw.decode().strip() == f"event: {name}":
                    return True
                if time.time() > deadline:
                    break
    except Exception:  # noqa: BLE001 — a timeout means no event came
        pass
    return False


def spoken(ctx: Ctx) -> None:
    uid, _e, tok = ctx.make_family_admin("listener", PW)
    fid = ctx.call("GET", "/api/v2/me", tok)["family"]["id"]
    show, eid = _upload_episode(ctx, tok)
    if ctx.feature("translation"):
        got = ctx.request("POST", f"/api/v2/translation/episodes/{eid}/quote", tok, {"target_language": "cs", "voice_profile_id": "x"})
        ctx.check("translation: an episode without a transcript cannot be quoted (400 problem)", _problem(got, 400), str(got[:2]))
    if not ctx.db_available:
        ctx.skip("translations as a kind, questions paged", "needs --compose-dir (a translation is paid work; one is made through SQL)")
        return
    if ctx.feature("translation"):
        translations(ctx, uid, fid, tok, show, eid)
    if ctx.feature("summaries"):
        questions(ctx, uid, fid, tok, eid)


def translations(ctx: Ctx, uid: str, fid: str, tok: str, show: str, eid: str) -> None:
    tid = str(uuid.uuid4())
    cursor = ctx.call("GET", "/api/v2/sync/changes", tok)["cursor"]
    _translation_row(ctx, tid, uid, fid, eid, "complete")

    info = ctx.request("POST", "/api/v2/playback-info", tok, {"kind": "translation", "id": tid})
    url = _json(info).get("url", "")
    ctx.check("POST /playback-info plays a translation: a signed link", info[0] == 200 and "/api/v2/stream" in url, str(info[:2]))
    if url:
        st, _h, _b = ctx.request("GET", url, None)
        ctx.check("…which serves the translation's audio", st in (200, 206), str(st))

    put = ctx.call("PUT", f"/api/v2/me/progress/translation/{tid}", tok, {"position_ms": 5000})
    ctx.check("PUT /me/progress/translation/{id}: the position is kept",
              put.get("applied") is True and put["progress"].get("kind") == "translation" and put["progress"].get("position_ms") == 5000, str(put))
    old = ctx.call("PUT", f"/api/v2/me/progress/translation/{tid}", tok, {"position_ms": 1000, "at": "2020-01-01T00:00:00Z"})
    ctx.check("…an older position does not win", old.get("applied") is False and old["progress"].get("position_ms") == 5000, str(old))
    alias = ctx.call("PUT", f"/api/v2/translation/episodes/{tid}/progress", tok, {"position_secs": 7.5, "completed": False})
    ctx.check("the apps' PUT /translation/episodes/{id}/progress answers in seconds and milliseconds",
              alias.get("progress_secs") == 7.5 and alias.get("progress_ms") == 7500 and alias.get("duration_ms") == 4000, str(alias)[:300])
    got = ctx.call("GET", f"/api/v2/me/progress/translation/{tid}", tok)
    ctx.check("…and writes the one progress model: /me/progress reads it", got.get("position_ms") == 7500, str(got))
    listed = ctx.call("GET", "/api/v2/me/progress?kind=episode,translation&unfinished=true", tok)
    ctx.check("GET /me/progress?kind=episode,translation lists it", any(i["id"] == tid and i["kind"] == "translation" for i in listed["items"]),
              str(listed)[:300])

    changes = ctx.call("GET", f"/api/v2/sync/changes?cursor={urllib.parse.quote(cursor)}&kinds=translation,progress.translation", tok)
    seen = {(c["kind"], c["id"]): c for c in changes["changes"]}
    record = (seen.get(("translation", tid)) or {}).get("record") or {}
    progress = (seen.get(("progress.translation", tid)) or {}).get("record") or {}
    ctx.check("the change feed: the translation and where the listener is in it",
              record.get("status") == "complete" and record.get("episode_id") == eid and record.get("duration_ms") == 4000
              and progress.get("position_ms") == 7500, str(changes)[:400])

    recent = ctx.call("GET", "/api/v2/translation/recent", tok)
    mine = [r for r in recent if r.get("id") == tid] if isinstance(recent, list) else []
    ctx.check("GET /translation/recent: the shipped apps' array, with milliseconds",
              len(mine) == 1 and mine[0].get("duration_ms") == 4000 and mine[0].get("progress_ms") == 7500, str(recent)[:300])
    page = ctx.call("GET", "/api/v2/translation/recent?limit=1", tok)
    ctx.check("…paged when asked: {items, next}", isinstance(page, dict) and len(page.get("items", [])) == 1, str(page)[:200])
    feed = ctx.call("GET", f"/api/v2/translation/feeds/{show}", tok)
    feed_page = ctx.call("GET", f"/api/v2/translation/feeds/{show}?limit=10", tok)
    ctx.check("GET /translation/feeds/{id}: an array, or a page when asked",
              isinstance(feed, list) and any(t["id"] == tid for t in feed)
              and isinstance(feed_page, dict) and any(t["id"] == tid for t in feed_page.get("items", [])), str(feed_page)[:200])

    by_episode = ctx.call("GET", f"/api/v2/translation/episodes/{eid}", tok)
    count = ctx.call("GET", f"/api/v2/translation/feeds/{show}/count", tok)
    ctx.check("GET /translation/episodes/{id} and the show's count know it",
              any(t["id"] == tid and t.get("progress_ms") == 7500 for t in by_episode) and count.get("count") == 1,
              f"{str(by_episode)[:200]} {count}")

    _sid, _se, stranger = ctx.make_family_admin("stranger", PW)
    ctx.check("another household: no playback-info (404)",
              _problem(ctx.request("POST", "/api/v2/playback-info", stranger, {"kind": "translation", "id": tid}), 404))
    ctx.check("…no progress to read or write (404)",
              _problem(ctx.request("GET", f"/api/v2/me/progress/translation/{tid}", stranger), 404)
              and _problem(ctx.request("PUT", f"/api/v2/me/progress/translation/{tid}", stranger, {"position_ms": 1}), 404))

    second = str(uuid.uuid4())
    came = _event_after(ctx, tok, "translation", lambda: _translation_row(ctx, second, uid, fid, eid, "translating"))
    ctx.check("a translation that starts is told on /events", came)

    ctx.call("DELETE", f"/api/v2/me/progress/translation/{tid}", tok, expect=(204,))
    ctx.check("…and starting over forgets the position", _problem(ctx.request("GET", f"/api/v2/me/progress/translation/{tid}", tok), 404))


def questions(ctx: Ctx, uid: str, fid: str, tok: str, eid: str) -> None:
    for i in range(3):
        ctx.sql(f"""INSERT INTO hosted.episode_questions (id, episode_id, family_id, user_id, question, answer, model,
                                                          input_tokens, output_tokens, created_at)
                    VALUES ('{uuid.uuid4()}', '{eid}', '{fid}', '{uid}', 'q{i}', 'a{i}', 'conformance', 1, 1,
                            now() - interval '{3 - i} minutes')""")
    ctx.on_cleanup("drop the questions", lambda: ctx.sql(f"DELETE FROM hosted.episode_questions WHERE episode_id = '{eid}'"))
    whole = ctx.call("GET", f"/api/v2/summaries/episodes/{eid}/questions", tok)
    first = ctx.call("GET", f"/api/v2/summaries/episodes/{eid}/questions?limit=2", tok)
    rest = ctx.call("GET", f"/api/v2/summaries/episodes/{eid}/questions?limit=2&cursor={urllib.parse.quote(first.get('next') or '')}", tok) \
        if first.get("next") else {"items": []}
    ids = [q["id"] for q in whole.get("items", [])]
    paged = [q["id"] for q in first.get("items", []) + rest.get("items", [])]
    ctx.check("questions: all of them without a limit, oldest first",
              [q["question"] for q in whole.get("items", [])] == ["q0", "q1", "q2"] and not whole.get("next"), str(whole)[:200])
    ctx.check("…paged two at a time without loss or repeats", paged == ids and not rest.get("next"), f"{paged} against {ids}")
    ctx.check("…each with what it cost in minor units and its currency",
              all(q.get("charged_minor") == 0 and q.get("currency") == "USD" for q in whole.get("items", [])), str(whole)[:200])
