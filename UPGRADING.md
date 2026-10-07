# Upgrading

## The server

```bash
docker compose exec backup sh /db-backup.sh now   # a fresh dump first
docker compose pull
docker compose up -d
```

Migrations run on start and are forward-only: do not go back to an older
image once a newer one has started. Skipping versions is fine. What changed
is in `CHANGELOG.md`; anything you have to do yourself is called out there.

From a version before 1.0.0-alpha.8: the server now runs as `PUID:PGID`
(1000:1000 by default) instead of root. It takes over its own storage at the
first start by itself; check that it can still read your library folders
(`INSTALL.md`, "Library folders").

## PostgreSQL

A server update never changes PostgreSQL: the compose file pins its major
version (16). Moving to a new major is a separate, deliberate step, because
PostgreSQL's files are not compatible between majors and need a dump and a
restore. Nothing forces you to do it soon; 16 is supported until late 2028.

```bash
scripts/pg-upgrade.sh 17
```

The script stops the server, dumps the database into `backups/`, starts
PostgreSQL 17 on a new data directory beside the old one, restores, compares
the number of rows in every table, and starts the server again. If anything
differs it stops and tells you how to go back. The old data directory stays
until you remove it (the script prints the command); going back is setting
`POSTGRES_MAJOR` and `PGDATA_DIR` in `.env` to their old values.

It works on the compose stack from this repository. With your own
PostgreSQL, use that server's own upgrade procedure (`pg_upgrade`, or a dump
and restore as in `BACKUP.md`).
