<p align="center">
  <img src="brand/own-audio-mark.svg" width="96" alt="own.audio">
</p>

# own.audio server

A self-hosted server for a family's **audiobooks, podcasts and music** — one
library, one server, native apps for Mac, iPhone, iPad, Apple TV, Android and
Windows, plus a web console. Rust and PostgreSQL. Free software under the
GNU Affero General Public License, version 3 or later.

> **Status: pre-release.** The server code is here (a snapshot of the
> codebase behind the hosted service at [own.audio](https://www.own.audio),
> with the hosted-only parts left out) and builds from source with the compose
> stack below; there is no tagged release or published image yet, and the
> install guide is still being reworked. The plan, the scope and the API
> policy are the place to start:
>
> - [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) — what happens, in what order
> - [docs/SCOPE.md](docs/SCOPE.md) — what is in, what is optional, what stays hosted-only
> - [docs/API_COMPATIBILITY.md](docs/API_COMPATIBILITY.md) — how the API is versioned so every client works with every server
> - [docs/LICENSING.md](docs/LICENSING.md) — why AGPL

## What it does

- **Audiobooks**: upload or index in place, multi-file books, authors,
  series, collections, bookmarks, progress that follows you between devices.
- **Podcasts**: subscribe, refresh, download on the server, transcripts
  where the feed has them, YouTube channels as feeds.
- **Music**: tags-based library, albums and artists, playlists, smart
  playlists, lyrics, stars, duplicate detection; identify against MusicBrainz;
  an OpenSubsonic API so your favourite music app works too.
- **Family**: one server, several people. Private by default, shared when you
  say so; roles, parental controls per member, join by link or QR code.
- **Your files stay yours**: point the server at the music and audiobook
  folders you already have on a disk or NAS. It indexes them read-only and
  never writes there. Uploads go to local storage or any S3-compatible bucket.
- **Playback everywhere**: progress, bookmarks and the play queue sync across
  the native apps and the web console; a 30-day trash catches mistakes.
- **Nothing phones home.** No telemetry, no update checks, no accounts
  anywhere but on your server.

Optional, off until you configure them: sign-in with Google, Apple or
Microsoft; e-mail notifications over SMTP; book identification through
Google Books; podcast discovery through Podcast Index.

## Two editions, one API

The hosted service at own.audio runs this same code plus a private layer. A
client cannot tell them apart and does not need to: it asks the server what
it offers.

| | This server | own.audio hosted |
|---|---|---|
| Run it on your own hardware | yes | — |
| Audiobooks, podcasts, music, family sharing, web console, OpenSubsonic | yes | yes |
| Read-only library folders, local storage | yes | — |
| Sign-in providers, SMTP mail, music identify | optional, you configure | yes |
| Narrate a book into an audiobook, translate a podcast episode | — | yes, metered |
| Storage billing, payments | — | yes |

## Running it

```bash
git clone https://github.com/own-audio/server.git && cd server
cp .env.example .env            # set POSTGRES_PASSWORD, S3_SECRET_KEY, SESSION_SECRET
docker compose pull server      # amd64: the published pre-release image
docker compose build server     # arm64 (Raspberry Pi, Apple silicon): build it, ~15 min
docker compose up -d            # PostgreSQL 16, RustFS and the server
```

Then open `http://localhost:8080` and create the first admin. The image is
`ghcr.io/own-audio/server` (also `kornelko2/own-audio-server` on Docker Hub);
amd64 and arm64. [INSTALL.md](INSTALL.md) has the details.

## Clients

The native apps live in their own repositories and are not part of this one.
They work against this server and against the hosted service alike; the
minimum server version each needs is stated in its README. Any OpenSubsonic
client works with the music library.

## Project

This is one person's project, built first for one family and published so
that others can run it too. Issues are welcome and read. Pull requests will
be accepted once the contributor agreement is in place; until then they are
not merged, for the reasons in [docs/LICENSING.md](docs/LICENSING.md).
Releases happen when they are ready. There is no SLA and no roadmap promise
beyond what the plan says.

Security problems: please report them privately, see `SECURITY.md` once it
exists; until then, e-mail contact@own.audio.

## Licence

[AGPL-3.0-or-later](LICENSE). You may run, study, change and share this
server, commercially included. If you offer a modified version to others
over a network, you must offer them your changes under the same licence.

The own.audio name and mark are trademarks and are not covered by the
licence. Unmodified builds may show them; a fork must use its own — see
[TRADEMARK.md](TRADEMARK.md) and [brand/](brand/).
