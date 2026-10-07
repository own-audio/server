# Running the server on a Raspberry Pi

A Raspberry Pi 3 (1 GB) is enough for a family: the server needs about
20 MiB at rest and PostgreSQL, sized for a family, about 30 MiB
(`RAM_USAGE.md`). A Pi 4 or 5 is faster at the first scan of a big library,
not required.

## What you need

- **Raspberry Pi 3, 4 or 5** with the **64-bit** Raspberry Pi OS Lite. The
  image is `linux/arm64`; a 32-bit OS cannot run it, and neither can an
  original Pi 2 (its v1.2 board has the Pi 3's chip and works).
- **A USB disk or SSD** for the database and your audio. An SD card works,
  but PostgreSQL writes to it all day and wears it out; keep the card for the
  system only.
- Docker with the compose plugin:

  ```bash
  curl -fsSL https://get.docker.com | sh
  sudo usermod -aG docker $USER   # log out and back in
  ```

## The disk

Mount the disk, e.g. at `/mnt/media`, with your collections on it:

```
/mnt/media/music/        Artist/Album/01 Song.flac …
/mnt/media/audiobooks/   Author/Title/01.mp3 … cover.jpg
/mnt/media/own-audio/    created by the server: uploads, podcast downloads
/mnt/media/postgres/     created by PostgreSQL
```

## The stack

```bash
git clone https://github.com/own-audio/server.git && cd server
cp .env.example .env    # set POSTGRES_PASSWORD and SESSION_SECRET
```

`docker-compose.override.yml`, next to `docker-compose.yml`:

```yaml
services:
  postgres:
    volumes:
      - /mnt/media/postgres:/var/lib/postgresql/data
  server:
    environment:
      LIBRARY__MUSIC: /music
      LIBRARY__AUDIOBOOKS: /audiobooks
    volumes:
      - /mnt/media/own-audio:/data/media
      - /mnt/media/music:/music:ro
      - /mnt/media/audiobooks:/audiobooks:ro
```

```bash
docker compose pull
docker compose up -d
```

Open `http://<the Pi's address>:8080`, create the first admin, and the scan of
your folders starts. A first scan reads every file's tags, so a large
collection takes a while on a Pi (figures from a Pi 3 will be added here);
`GET /api/v1/library/folders` shows how far it is. Later scans only read what
changed.

Set `PUBLIC_URL` in `.env` to the address the family uses — media links are
built from it.

## Reaching it from outside

The simplest way that opens no port on your router is a Cloudflare Tunnel:
create a tunnel in the Cloudflare dashboard (Zero Trust → Networks →
Tunnels), point a public hostname at `http://server:8080`, and add the
connector next to the server:

```yaml
services:
  cloudflared:
    image: cloudflare/cloudflared:latest
    command: tunnel --no-autoupdate run
    environment:
      TUNNEL_TOKEN: ${TUNNEL_TOKEN}
    restart: unless-stopped
```

Put `TUNNEL_TOKEN` in `.env`, set `PUBLIC_URL=https://your.hostname`, and
`SERVER__RATE_LIMIT__TRUST_PROXY_HEADERS=true` so the rate limits see the
real client addresses. Audio streams through the tunnel from the Pi, so your
upload speed is the limit for listening away from home.
