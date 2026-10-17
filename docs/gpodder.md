# gPodder sync

Podcast apps that sync with a gPodder server — [AntennaPod](https://antennapod.org),
[Kasts](https://apps.kde.org/kasts/), [gPodder](https://gpodder.github.io),
own.audio Podcasts and others that speak gpodder.net's API — can use this
server for it. Your shows and where you are in each episode then follow you
between those apps and the own.audio apps.

## Connecting an app

| The app asks for | Enter |
|---|---|
| Server | this server's address, e.g. `https://audio.example.com` |
| User name | your email |
| Password | the key under **Settings → Other apps** in the console (the same key Subsonic music apps use) |

Your account password is never used here: an app keeps the key and sends it
with every request, and a key can be replaced at any time (**Generate a new
one**) without touching your sign-in.

In AntennaPod: *Settings → Synchronization → gpodder.net*, then *Custom server*.

## What is synchronised

- **Subscriptions.** Your subscriptions are the podcasts you own on this
  server (not shows someone else in the family shares with you). A show you
  add in a gPodder app becomes one of your podcasts, private to you until you
  share it; the server reads its title, artwork and episodes within a minute.
  A show you add or remove in the console, an own.audio app or a Subsonic app
  reaches your gPodder apps on their next sync. A show the family already
  shares with you is not copied when an app adds it.
- **Positions and played episodes.** A position an app reports becomes your
  position in that episode when one of your visible podcasts has it (matched
  by the episode's file address, or its GUID); the newer of two positions
  wins. Within the last 30 seconds or past 98% counts as played. Positions you
  reach in the own.audio apps are handed to gPodder apps the same way.
- **Devices.** Each app registers itself; subscriptions are the same on every
  device.

Not synchronised: the gPodder web site's own features (lists, suggestions,
favourites, settings).

## For developers

The API is gpodder.net's [API v2](https://gpoddernet.readthedocs.io/en/latest/api/reference/),
JSON only, at `/api/2`, with HTTP basic authentication on every request:

| Method | Path | |
|---|---|---|
| POST | `/api/2/auth/{email}/login.json` | checks the credentials; `logout.json` too |
| GET | `/api/2/devices/{email}.json` | `[{id, caption, type, subscriptions}]` |
| POST | `/api/2/devices/{email}/{device}.json` | `{caption, type}` |
| GET | `/api/2/subscriptions/{email}/{device}.json?since=` | `{add, remove, timestamp}` |
| POST | `/api/2/subscriptions/{email}/{device}.json` | `{add, remove}` → `{timestamp, update_urls}` |
| GET | `/api/2/episodes/{email}.json?since=&podcast=&device=&aggregated=` | `{actions, timestamp}` |
| POST | `/api/2/episodes/{email}.json` | `[action…]` → `{timestamp, update_urls}` |

- `timestamp` is unix seconds. Pass the last one back as `since`; answers
  overlap the previous one by a few seconds, and repeating an action is
  harmless. A subscription change is not handed back to the device that made
  it.
- Actions are `download`, `play` (with `started`, `position`, `total` in
  seconds), `delete` and `new`; times are UTC in the API's form
  `2026-10-17T08:00:00`. A `play` or `new` changes the listener's progress;
  `download` and `delete` are kept for the other apps only.
- `update_urls` lists `[sent, kept]` pairs for addresses the server cleaned up
  (spaces, `feed://` links); an empty `kept` means it was refused.
- A `{email}` other than the signed-in account answers `403`; wrong
  credentials `401` with a `Basic` challenge. `GET /api/v1/server` reports
  `features.gpodder` (contract revision 16).
