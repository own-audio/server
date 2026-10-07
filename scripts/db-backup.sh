#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Nightly PostgreSQL dumps for the compose stack (the `backup` service).
#
#   db-backup.sh loop   wait for BACKUP_HOUR (UTC) every day; dump at start too
#                       when the newest dump is older than a day
#   db-backup.sh now    one dump, right away
#
# Dumps are pg_dump custom format in /backups/daily, one per day; the Sunday
# one is also kept in /backups/weekly. KEEP_DAILY (7) and KEEP_WEEKLY (4)
# newest survive. /backups/last-success holds the time of the last good dump.
# Restore: BACKUP.md.
set -eu

DIR=/backups
KEEP_DAILY=${KEEP_DAILY:-7}
KEEP_WEEKLY=${KEEP_WEEKLY:-4}
BACKUP_HOUR=${BACKUP_HOUR:-3}

prune() {
    # Names sort by date; keep the newest $2 in $1.
    ls -1 "$1"/*.dump 2>/dev/null | sort -r | tail -n +"$(($2 + 1))" | while read -r old; do
        rm -f "$old"
    done
}

dump() {
    mkdir -p "$DIR/daily" "$DIR/weekly"
    day=$(date -u +%F)
    out="$DIR/daily/own-audio-$day.dump"
    # Written aside and moved, so a dump cut short never looks like a good one.
    pg_dump -Fc -f "$out.part"
    mv "$out.part" "$out"
    if [ "$(date -u +%u)" = 7 ]; then
        cp "$out" "$DIR/weekly/own-audio-$day.dump"
    fi
    prune "$DIR/daily" "$KEEP_DAILY"
    prune "$DIR/weekly" "$KEEP_WEEKLY"
    date -u +%FT%TZ > "$DIR/last-success"
    echo "$(date -u +%FT%TZ) backup: $out ($(du -h "$out" | cut -f1))"
}

try_dump() {
    dump || echo "$(date -u +%FT%TZ) backup FAILED; next attempt at the next scheduled hour" >&2
}

seconds_until_hour() {
    now=$(date -u +%s)
    target=$(date -u -d "$(date -u +%F) $BACKUP_HOUR:00:00" +%s 2>/dev/null \
        || date -u -D '%Y-%m-%d %H:%M:%S' -d "$(date -u +%F) $BACKUP_HOUR:00:00" +%s)
    [ "$target" -le "$now" ] && target=$((target + 86400))
    echo $((target - now))
}

case "${1:-loop}" in
    now)
        dump
        ;;
    loop)
        newest=$(ls -1 "$DIR"/daily/*.dump 2>/dev/null | sort -r | head -1 || true)
        if [ -z "$newest" ] || [ -n "$(find "$newest" -mmin +1440 2>/dev/null)" ]; then
            try_dump
        fi
        while true; do
            sleep "$(seconds_until_hour)"
            try_dump
        done
        ;;
    *)
        echo "usage: db-backup.sh [loop|now]" >&2
        exit 2
        ;;
esac
