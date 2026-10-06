# Installing the own.audio server

> Pre-release (`1.0.0-alpha.1`). The published image is
> `ghcr.io/own-audio/server`, amd64 only for now; on arm64 build from source
> with `docker compose build server`.

## What you need

- A Linux host (amd64 or arm64) with Docker and Docker Compose v2. A Raspberry
  Pi 4, a NAS that runs containers, or a small VPS all work.
- PostgreSQL 16 or newer. The compose file brings its own; point
  `DATABASE_URL` at an existing one if you prefer.
- Somewhere to keep the audio. The compose file brings RustFS, an
  S3-compatible object store; any S3-compatible store works, and a plain
  local directory is coming (see the plan, Phase 4).
- A hostname and HTTPS in front of it if anyone connects from outside your
  network. The server speaks plain HTTP on one port; put Caddy, nginx or a
  tunnel in front.

## Quick start

```bash
git clone https://github.com/own-audio/server.git
cd server
cp .env.example .env
# set POSTGRES_PASSWORD, S3_SECRET_KEY and SESSION_SECRET (openssl rand -hex 32)
docker compose pull server      # amd64; on arm64 run `docker compose build server` instead
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
| `S3_PUBLIC_ENDPOINT` | The address browsers and apps reach the object store at. Media is fetched straight from the store through signed URLs, so this must be reachable from every device. | `http://localhost:9000` |
| `SESSION_SECRET` | Signs sessions. Changing it signs everyone out. | required |
| `SERVER__RATE_LIMIT__*` | Per-IP limits on login, refresh, device codes, join codes and setup. `…__ENABLED=false` turns them off; `…__TRUST_PROXY_HEADERS=true` when you are behind a proxy on a public address. | on |
| `AUTH__GOOGLE__*`, `AUTH__APPLE__*`, `AUTH__MICROSOFT__*` | Sign-in providers. Off until you set client ids and `…__ENABLED=true`. | off |
| `MAIL__*` | SMTP for invite and notification mail. Off until set; invites work by link and QR without it. | off |
| `METADATA__BASE_URL` | A music-metadata service for identify and podcast discovery. Without it the open-source edition uses the public MusicBrainz API (Phase 4). | unset |

The full list is `backend/src/app/config.rs`; every field reads from the
environment with `__` between levels.

## Upgrading

```bash
docker compose pull
docker compose up -d
```

Migrations run on start and are forward-only: back up before upgrading and do
not downgrade. Skipping versions is fine.

## Backup

Two things hold your data: the PostgreSQL database and the object store's
bucket. Back up both, together:

```bash
docker compose exec -T postgres pg_dump -U ownaudio ownaudio | gzip > ownaudio-$(date +%F).sql.gz
# the bucket: rclone, mc, or a snapshot of the rustfs_data volume
```

## Where things are

- Server: port 8080, `/health` for liveness, `/api/v1/server` for what this
  server offers, the web console at `/`.
- Object store console: `http://localhost:9001/rustfs/console/`.
- Logs: `docker compose logs -f server`.
