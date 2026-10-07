#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Runs the server as PUID:PGID (default 1000:1000) instead of root.
set -e

if [ "$(id -u)" != 0 ]; then
    # Already started as a non-root user (`docker run --user`, compose `user:`).
    exec /app/audio2 "$@"
fi

PUID=${PUID:-1000}
PGID=${PGID:-1000}

# Installs from before 1.0.0-alpha.8 wrote /data/media as root. Hand it over
# once; afterwards the owner already matches and nothing is walked.
mkdir -p /data/media
if [ "$(stat -c %u:%g /data/media)" != "$PUID:$PGID" ]; then
    chown -R "$PUID:$PGID" /data/media
fi

export HOME=/tmp
exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups /app/audio2 "$@"
