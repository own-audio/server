#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Move the compose stack's database to a new PostgreSQL major version.
#
#   scripts/pg-upgrade.sh 17
#
# Run it in the directory with docker-compose.yml and .env. It dumps the
# database, starts the new major on an empty data directory beside the old
# one (same volume), restores the dump, compares the row count of every table
# and only then starts the server again. The old data directory is left
# untouched: going back is two lines in .env (printed at the end).
set -eu

NEW=${1:?usage: scripts/pg-upgrade.sh <new major, e.g. 17>}
case "$NEW" in *[!0-9]*|'') echo "the major is a number, e.g. 17" >&2; exit 2 ;; esac

# COMPOSE and ENV_FILE only for a stack started with other options.
ENV_FILE=${ENV_FILE:-.env}
dc() { ${COMPOSE:-docker compose} "$@"; }
env_get() { sed -n "s/^$1=//p" "$ENV_FILE" 2>/dev/null | tail -1; }
env_set() {
    if grep -q "^$1=" "$ENV_FILE" 2>/dev/null; then
        sed -i.bak "s#^$1=.*#$1=$2#" "$ENV_FILE" && rm -f "$ENV_FILE.bak"
    else
        printf '%s=%s\n' "$1" "$2" >> "$ENV_FILE"
    fi
}

U=$(env_get POSTGRES_USER); U=${U:-ownaudio}
DB=$(env_get POSTGRES_DB); DB=${DB:-ownaudio}
OLD_DATA=$(env_get PGDATA_DIR); OLD_DATA=${OLD_DATA:-/var/lib/postgresql/data/pgdata}
OLD_MAJOR=$(dc exec -T postgres sh -c 'postgres -V' | sed 's/[^0-9]*\([0-9]*\).*/\1/')
[ -n "$OLD_MAJOR" ] || { echo "is the stack running? (docker compose up -d postgres)" >&2; exit 1; }
if [ "$NEW" -le "$OLD_MAJOR" ]; then
    echo "already on PostgreSQL $OLD_MAJOR; nothing to do" >&2; exit 1
fi
NEW_DATA=/var/lib/postgresql/data/pg$NEW

COUNTS="SELECT table_name || ' ' || (xpath('/row/c/text()', query_to_xml(format('SELECT count(*) AS c FROM %I.%I', table_schema, table_name), false, true, '')))[1]::text
        FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY 1"

echo "PostgreSQL $OLD_MAJOR -> $NEW"
echo "1/5 stopping the server so nothing changes during the copy"
dc stop server

mkdir -p backups
DUMP=backups/pre-upgrade-pg$OLD_MAJOR-$(date -u +%Y%m%dT%H%M%SZ).dump
echo "2/5 dumping to $DUMP"
dc exec -T postgres pg_dump -U "$U" -Fc "$DB" > "$DUMP"
dc exec -T postgres psql -U "$U" -d "$DB" -At -c "$COUNTS" > "$DUMP.counts"

echo "3/5 starting PostgreSQL $NEW in $NEW_DATA"
dc stop postgres
env_set POSTGRES_MAJOR "$NEW"
env_set PGDATA_DIR "$NEW_DATA"
dc up -d postgres
i=0
until dc exec -T postgres pg_isready -U "$U" -d "$DB" >/dev/null 2>&1; do
    i=$((i + 1)); [ $i -gt 60 ] && { echo "PostgreSQL $NEW did not start" >&2; exit 1; }
    sleep 2
done

echo "4/5 restoring"
dc exec -T postgres pg_restore -U "$U" -d "$DB" --no-owner --exit-on-error < "$DUMP"
dc exec -T postgres psql -U "$U" -d "$DB" -At -c "$COUNTS" > "$DUMP.counts-after"
dc exec -T postgres psql -U "$U" -d "$DB" -c "ANALYZE" >/dev/null

if ! cmp -s "$DUMP.counts" "$DUMP.counts-after"; then
    echo "ROW COUNTS DIFFER; the server stays stopped:" >&2
    diff "$DUMP.counts" "$DUMP.counts-after" >&2 || true
    echo "To go back: set POSTGRES_MAJOR=$OLD_MAJOR and PGDATA_DIR=$OLD_DATA in .env, then docker compose up -d" >&2
    exit 1
fi

echo "5/5 every table has the same number of rows; starting the server"
dc up -d
echo
echo "Done: PostgreSQL $NEW. The dump stays in $DUMP."
echo "The old data directory ($OLD_DATA) is still in the volume. To go back,"
echo "set POSTGRES_MAJOR=$OLD_MAJOR and PGDATA_DIR=$OLD_DATA in .env and run docker compose up -d."
echo "Once you are happy, free its space with:"
echo "  docker compose exec postgres rm -rf $OLD_DATA"
