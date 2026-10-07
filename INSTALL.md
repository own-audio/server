# Installing the own.audio server

> Pre-release (`1.0.0-alpha.5`). The published image is
> `ghcr.io/own-audio/server` (also `kornelko2/own-audio-server` on Docker
> Hub), for amd64 and arm64.

## What you need

- A Linux host (amd64 or arm64) with Docker and Docker Compose v2. A Raspberry
  Pi 4, a NAS that runs containers, or a small VPS all work.
- PostgreSQL 16 or newer. The compose file brings its own; point
  `DATABASE_URL` at an existing one if you prefer.
- Somewhere to keep the audio. By default it is a volume on the same host
  (`media_data`), served by the server itself — nothing else to run. An
  S3-compatible store (RustFS is bundled, or any other) is the alternative,
  see "Storage" below. Indexing folders you already have, read-only, is
  coming (issue #1).
- A hostname and HTTPS in front of it if anyone connects from outside your
  network. The server speaks plain HTTP on one port; put Caddy, nginx or a
  tunnel in front.

## Quick start

```bash
git clone https://github.com/own-audio/server.git
cd server
cp .env.example .env
# set POSTGRES_PASSWORD and SESSION_SECRET (openssl rand -hex 32)
docker compose pull server
docker compose up -d
```

Open `http://localhost:8080`. The first screen creates the admin account;
that account owns the first family. Invite the others from the Family page by
link or QR code — there is no open sign-up unless you turn it on
(`AUTH__REGISTRATION_OPEN=true`).

## Settings that matter

| Variable | What it is | Default |
|---|---|---|
| `PUBLIC_URL` | The address people and apps reach the server at. Links in invites and e-mails are built from it. | `http://localhost:8080` |
| `STORAGE_KIND` | `local`: media in the `media_data` volume, streamed by the server. `s3`: an S3-compatible store, see "Storage". | `local` |
| `SESSION_SECRET` | Signs sessions and media links. Changing it signs everyone out and ends open media links. | required |
| `SERVER__RATE_LIMIT__*` | Per-IP limits on login, refresh, device codes, join codes and setup. `…__ENABLED=false` turns them off; `…__TRUST_PROXY_HEADERS=true` when you are behind a proxy on a public address. | on |
| `AUTH__GOOGLE__*`, `AUTH__APPLE__*`, `AUTH__MICROSOFT__*` | Sign-in providers. Off until you set client ids and `…__ENABLED=true`. | off |
| `MAIL__*` | SMTP for invite and notification mail. Off until set; invites work by link and QR without it. | off |
| `METADATA__BASE_URL` | A music-metadata service for identify and podcast discovery. Without it the open-source edition uses the public MusicBrainz API (Phase 4). | unset |

The full list is `backend/src/app/config.rs`; every field reads from the
environment with `__` between levels.

## Storage

**Local (default).** Every upload is a file under `/data/media` in the server
container, kept in the `media_data` volume. Players get links to the server's
own `/api/v1/media` route, signed by the server and valid for 4 hours, with
seeking supported. Mount a host folder there instead of the volume if you
want the files on a particular disk:

```yaml
services:
  server:
    volumes:
      - /mnt/music-disk/own-audio:/data/media
```

**S3-compatible.** Set `STORAGE_KIND=s3`. For the bundled RustFS also set
`COMPOSE_PROFILES=s3` and `S3_SECRET_KEY`; for another store set
`S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY` and `S3_SECRET_KEY`. Players then
fetch media straight from the store through presigned links, so
`S3_PUBLIC_ENDPOINT` must be an address every device can reach. If some
networks block the store's host (company firewalls often block cloud-storage
domains), set `S3_PROXY=true` and the server streams the media itself, as
with local storage.

Switching an existing install from one kind to the other does not move the
files; start fresh or copy the objects across by their keys.

## Upgrading

```bash
docker compose pull
docker compose up -d
```

Migrations run on start and are forward-only: back up before upgrading and do
not downgrade. Skipping versions is fine.

## Backup

Two things hold your data: the PostgreSQL database and the media. Back up
both, together:

```bash
docker compose exec -T postgres pg_dump -U ownaudio ownaudio | gzip > ownaudio-$(date +%F).sql.gz
# local storage: the media_data volume (or the folder mounted at /data/media)
# s3: the bucket, with rclone, mc, or a snapshot of the rustfs_data volume
```

## Where things are

- Server: port 8080, `/health` for liveness, `/api/v1/server` for what this
  server offers, the web console at `/`.
- Media: the `media_data` volume (local storage), or with the `s3` profile
  the RustFS console at `http://localhost:9001/rustfs/console/`.
- Logs: `docker compose logs -f server`.
