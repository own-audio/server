<p align="center">
  <img src="brand/own-audio-mark.svg" width="96" alt="own.audio">
</p>

# own.audio server

A self-hosted server for a family's **audiobooks, podcasts and music** — one
library, one server, a web console and an OpenSubsonic API, with native apps
for Apple devices, Android and Windows in development. Rust and PostgreSQL.
Free software under the GNU Affero General Public License, version 3 or later.

[![Demo](https://img.shields.io/badge/demo-demo.own.audio-7c5cff)](https://demo.own.audio)
[![Docker Hub](https://img.shields.io/docker/v/kornelko2/own-audio-server?label=docker%20hub&sort=semver)](https://hub.docker.com/r/kornelko2/own-audio-server)
[![Licence](https://img.shields.io/badge/licence-AGPL--3.0--or--later-blue)](LICENSE)

> **Status: pre-release.** This is the codebase behind the hosted service at
> [own.audio](https://www.own.audio), with the hosted-only parts left out.
> Alpha releases are tagged and published as images for amd64 and arm64
> (see [CHANGELOG.md](CHANGELOG.md)); some features below are still being
> built and are marked as such. The plan, the scope and the API policy are
> the place to start:
>
> - [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) — what happens, in what order
> - [docs/SCOPE.md](docs/SCOPE.md) — what is in, what is optional, what stays hosted-only
> - [docs/API_COMPATIBILITY.md](docs/API_COMPATIBILITY.md) — how the API is versioned so every client works with every server
> - [docs/LICENSING.md](docs/LICENSING.md) — why AGPL

## Who it is for

One install serves **one family**, as many people as it has, on a home server, a NAS
or a small VPS. It is built for big collections — a data-hoarder household
with hundreds of thousands of songs and a thousand audiobooks — and aims for
a smaller footprint than running Navidrome and Audiobookshelf side by side.
The server process needs about 20 MiB at rest. How far the code is from that
target, and what is left to do, is in [docs/CAPACITY.md](docs/CAPACITY.md)
and [docs/RAM_USAGE.md](docs/RAM_USAGE.md).

## What it does

- **Audiobooks**: upload or index in place, multi-file books, authors,
  series, collections, bookmarks, progress that follows you between devices.
- **Podcasts**: subscribe, refresh, download on the server, transcripts
  where the feed has them, YouTube channels as feeds.
- **Music**: tags-based library, albums, artists and genres, playlists, smart
  playlists, lyrics, stars and ratings, duplicate detection; an OpenSubsonic
  API so your favourite music app works too. Identify against the public
  MusicBrainz API is coming (Phase 4).
- **Family**: one server, several people. Private by default, shared when you
  say so; roles, parental controls per member, join by link or QR code.
- **Your files stay yours**: point the server at the music and audiobook
  folders you already have on a disk or NAS; it indexes them read-only and
  never writes there. Uploads are plain files on the server's own disk, or in
  any S3-compatible bucket if you prefer.
- **Playback everywhere**: progress, bookmarks and the play queue sync across
  the web console and the apps; a 30-day trash catches mistakes.
- **Statistics**: listening history per member and a yearly recap.
- **Nothing phones home.** No telemetry, no update checks, no accounts
  anywhere but on your server.

Optional, off until you configure them: sign-in with Google, Apple or
Microsoft; book identification through Google Books. Coming: e-mail
notifications over SMTP and podcast discovery through Podcast Index.

## Two editions, one API

The hosted service at own.audio runs this same code plus a private layer. A
client cannot tell them apart and does not need to: it asks the server what
it offers.

| | This server | own.audio hosted |
|---|---|---|
| Run it on your own hardware | yes | — |
| Audiobooks, podcasts, music, family sharing, web console, OpenSubsonic | yes | yes |
| Local storage | yes | — |
| Read-only library folders | yes | — |
| Sign-in providers | optional, you configure | yes |
| SMTP mail, music identify | coming, optional | yes |
| Narrate a book into an audiobook, translate a podcast episode | — | yes, metered |
| Storage billing, payments | — | yes |

## Try it first

A public demo of exactly this edition runs at **https://demo.own.audio**:
sign in as `guest@demo.own.audio` with the password `own-audio-demo`. It
holds four public-domain LibriVox audiobooks, 38 Creative Commons songs,
three Creative Commons podcasts and a year of the guest's listening, so the
statistics, the yearly recap and the smart playlists have something to show.
It resets every night, so change whatever you like.

## Running it

```bash
git clone https://github.com/own-audio/server.git && cd server
cp .env.example .env            # set POSTGRES_PASSWORD and SESSION_SECRET
docker compose pull             # the published pre-release image, amd64 or arm64
docker compose up -d            # PostgreSQL 16 and the server; media on a local volume
```

Then open `http://localhost:8080` and create the first admin. The image is
`ghcr.io/own-audio/server` (also `kornelko2/own-audio-server` on Docker Hub);
amd64 and arm64. [INSTALL.md](INSTALL.md) has the details.

## Clients

The native apps live in their own repositories and are not part of this one.
They are in development and not released yet. They are built to work against
this server and the hosted service alike. Today, any OpenSubsonic client
works with the music library, and the web console covers everything else.

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
