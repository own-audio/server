# Backup and restore

Your data is in three places. The server backs up the first by itself; the
other two are files you back up the way you back up anything else.

| What | Where | Backed up by |
|---|---|---|
| The database: accounts, the library's index, progress, playlists, history | PostgreSQL | the `backup` service, every night |
| Uploads and podcast downloads | the `media_data` volume (or your `/data/media` folder); with S3 the bucket | you: restic, rsync, a NAS snapshot, `rclone` for a bucket |
| Your library folders | your disk or NAS | you; the server only reads them |

## The nightly database dump

The `backup` service in `docker-compose.yml` dumps the database every night
at 03:00 UTC into `./backups` next to the compose file:

```
backups/daily/own-audio-2026-10-07.dump     the last 7 days
backups/weekly/own-audio-2026-10-04.dump    the last 4 Sundays
backups/last-success                        when the last dump finished
```

It also makes one right after it starts when the newest dump is more than a
day old. Settings in `.env`: `BACKUP_DIR` (another folder, e.g. a NAS mount),
`BACKUP_HOUR` (0–23, UTC). A dump of a family's database is small, usually a
few megabytes; it uses the same PostgreSQL image as the database, so its
version always matches.

A dump right now, before something risky:

```bash
docker compose exec backup sh /db-backup.sh now
```

`backups/` holds everything in the database, password hashes included. Keep
it as private as the server, and copy it off the machine with the rest.

## Restore

Into the running stack, replacing what is in the database:

```bash
docker compose stop server
docker compose exec -T postgres sh -c 'dropdb -U "$POSTGRES_USER" "$POSTGRES_DB" && createdb -U "$POSTGRES_USER" "$POSTGRES_DB"'
docker compose exec -T postgres sh -c 'pg_restore -U "$POSTGRES_USER" -d "$POSTGRES_DB" --no-owner --exit-on-error' \
    < backups/daily/own-audio-2026-10-07.dump
docker compose up -d
```

The server applies any newer migrations on start, so a dump from an older
version restores into a newer server. Not the other way round: migrations are
forward-only.

Restore the media from the same day as the dump, so the database does not
point at files that are not there. Files that are present but unknown to the
database are harmless.

On a new machine: install as in `INSTALL.md` with the same `.env` (the same
`SESSION_SECRET` keeps everyone signed in), start only PostgreSQL
(`docker compose up -d postgres`), then follow the steps above.
