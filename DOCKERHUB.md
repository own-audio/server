# own.audio server

A self-hosted server for a family's **audiobooks, podcasts and music**: one
library, one server, a web console, and an OpenSubsonic API for music apps.
Rust and PostgreSQL, free software under the AGPL-3.0-or-later.

**Pre-release.** The 1.0.0 alpha builds work and are tested, but the install
story is still settling. Read the
[implementation plan](https://github.com/own-audio/server/blob/main/docs/IMPLEMENTATION_PLAN.md)
before you rely on it.

**Try it without installing:** https://demo.own.audio, sign in as
`guest@demo.own.audio` with the password `own-audio-demo`. It resets every
night.

## What it does

- **Audiobooks**: multi-file books, authors, series, collections,
  bookmarks, progress that follows you between devices.
- **Podcasts**: subscribe, refresh and download on the server.
- **Music**: albums, artists, genres, playlists, smart playlists, stars and
  ratings, lyrics, duplicate detection. Any OpenSubsonic app can play it.
- **Family**: several people on one server. Private by default, shared when
  you say so, with roles and parental controls.
- **Statistics**: listening history and a yearly recap.
- **Nothing phones home**: no telemetry, no update checks.

## Quick start

The server needs PostgreSQL 16+ and an S3-compatible store. The compose file
in the repository brings both (PostgreSQL and RustFS):

```bash
git clone https://github.com/own-audio/server.git && cd server
cp .env.example .env    # set POSTGRES_PASSWORD, S3_SECRET_KEY, SESSION_SECRET
docker compose pull
docker compose up -d
```

Open `http://localhost:8080` and create the first admin. Put HTTPS in front
(Caddy, nginx or a tunnel) before anyone connects from outside your network.
The [install guide](https://github.com/own-audio/server/blob/main/INSTALL.md)
covers the settings, backups and upgrades.

## Tags

| Tag | What it is |
|---|---|
| `1.0.0-alpha.N` | A pre-release. Pin one of these. |
| `1.0`, `latest` | Will follow stable releases, from 1.0.0 on. |

Images are built for `linux/amd64` and `linux/arm64` by the repository's
release workflow. The same images are on `ghcr.io/own-audio/server`.

## API compatibility

Every server answers `GET /api/v1/server` with its version, its API contract
revision and the features it offers, so clients adapt instead of guessing.
The rules are in
[API_COMPATIBILITY.md](https://github.com/own-audio/server/blob/main/docs/API_COMPATIBILITY.md).

## Licence

[AGPL-3.0-or-later](https://github.com/own-audio/server/blob/main/LICENSE).
The own.audio name and mark are trademarks and not covered by the licence;
see [TRADEMARK.md](https://github.com/own-audio/server/blob/main/TRADEMARK.md).

Source, issues and changelog: https://github.com/own-audio/server
