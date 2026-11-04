#!/usr/bin/env bash
# Installs the own.audio server on this machine, once, with a few questions;
# run again later to upgrade. Everything lands in one folder.
#
#   curl -fsSL https://raw.githubusercontent.com/own-audio/server/main/install.sh | bash
#   ./install.sh                     # from a clone: uses the checkout's files
#
# Answers can be given up front, for scripts and for a second machine:
#   OWN_AUDIO_DIR=~/own-audio OWN_AUDIO_PORT=8080 OWN_AUDIO_MUSIC=/mnt/music \
#   OWN_AUDIO_AUDIOBOOKS=/mnt/audiobooks OWN_AUDIO_URL=http://192.168.1.20:8080 \
#   ./install.sh --yes
# OWN_AUDIO_VERSION picks a release (default: the newest); OWN_AUDIO_IMAGE another image.
set -euo pipefail

REPO="own-audio/server"
RAW="https://raw.githubusercontent.com/$REPO"
YES=0
for arg in "$@"; do
  case "$arg" in
    --yes|-y) YES=1 ;;
    --help|-h) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

# ── Output ──────────────────────────────────────────────────────────────────

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; RED=$'\033[31m'; RESET=$'\033[0m'
else
  BOLD=""; DIM=""; GREEN=""; RED=""; RESET=""
fi
say()  { printf '%s\n' "$*"; }
step() { printf '\n%s%s%s\n' "$BOLD" "$*" "$RESET"; }
ok()   { printf '%s✓%s %s\n' "$GREEN" "$RESET" "$*"; }
fail() { printf '%s✗ %s%s\n' "$RED" "$*" "$RESET" >&2; exit 1; }

# The script is usually piped from curl, so questions are read from the terminal.
ask() {  # ask VAR "question" "default"
  local var="$1" question="$2" default="$3" answer=""
  if [ "$YES" = 1 ] || ! [ -r /dev/tty ]; then
    printf -v "$var" '%s' "$default"
    return
  fi
  printf '%s%s%s' "$BOLD" "$question" "$RESET"
  [ -n "$default" ] && printf ' %s[%s]%s' "$DIM" "$default" "$RESET"
  printf ': '
  IFS= read -r answer < /dev/tty || true
  printf -v "$var" '%s' "${answer:-$default}"
}

confirm() {  # confirm "question" → 0 for yes
  local answer=""
  [ "$YES" = 1 ] && return 0
  [ -r /dev/tty ] || return 0
  printf '%s%s%s %s[Y/n]%s: ' "$BOLD" "$1" "$RESET" "$DIM" "$RESET"
  IFS= read -r answer < /dev/tty || true
  case "${answer:-y}" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# ── What this machine has ───────────────────────────────────────────────────

step "own.audio server — setup"
say "A server for your family's music, audiobooks and podcasts, in one folder on this machine."

command -v docker >/dev/null 2>&1 || fail "Docker is not installed. Linux: curl -fsSL https://get.docker.com | sh   macOS/Windows: Docker Desktop. Then run this again."
docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 is missing ('docker compose'). It comes with Docker Desktop and with the docker-compose-plugin package on Linux."
if ! docker info >/dev/null 2>&1; then
  if [ "$(uname -s)" = Linux ] && ! id -nG 2>/dev/null | grep -qw docker; then
    fail "Docker is installed but this user may not use it. Run: sudo usermod -aG docker $USER, then sign out and in, and run this again."
  fi
  fail "Docker is installed but not running. Start it and run this again."
fi
ok "Docker $(docker version --format '{{.Server.Version}}' 2>/dev/null || echo "is running")"

case "$(uname -m)" in
  x86_64|amd64|aarch64|arm64) ;;
  *) fail "The image is built for amd64 and arm64; this machine is $(uname -m)." ;;
esac

# ── Where ───────────────────────────────────────────────────────────────────

# From a clone, the checkout is the install folder and its files are used as they are
# (unless OWN_AUDIO_DIR says where else).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || true)"
if [ -z "${OWN_AUDIO_DIR:-}" ] && [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/docker-compose.yml" ] && [ -f "$SCRIPT_DIR/backend/Cargo.toml" ]; then
  FROM_CHECKOUT=1
  DIR="$SCRIPT_DIR"
else
  FROM_CHECKOUT=0
  ask DIR "Install folder" "${OWN_AUDIO_DIR:-$HOME/own-audio}"
  DIR="${DIR/#\~/$HOME}"
fi
mkdir -p "$DIR" || fail "Cannot create $DIR."
cd "$DIR"

# ── Upgrade, when it is already here ────────────────────────────────────────

if [ -f .env ] && [ -f docker-compose.yml ]; then
  step "Upgrading the server in $DIR"
  if [ "$FROM_CHECKOUT" = 0 ]; then
    VERSION="${OWN_AUDIO_VERSION:-$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)}"
    if [ -n "$VERSION" ]; then
      curl -fsSL "$RAW/v$VERSION/docker-compose.yml" -o docker-compose.yml.new && mv docker-compose.yml.new docker-compose.yml
      mkdir -p scripts && curl -fsSL "$RAW/v$VERSION/scripts/db-backup.sh" -o scripts/db-backup.sh
      ok "Release $VERSION"
    fi
  fi
  docker compose pull --quiet postgres server backup 2>&1 | grep -v -e Pulling -e Pulled || true
  docker compose up -d postgres server backup 2>&1 | grep -v -e Running -e Started -e Created -e Creating -e Starting -e Healthy -e Waiting || true
  URL="$(sed -n 's/^PUBLIC_URL=//p' .env | head -1)"
  say ""
  say "Waiting for the server…"
  for _ in $(seq 1 60); do
    if docker compose exec -T server curl -fsS http://localhost:8080/health >/dev/null 2>&1; then
      ok "Running $(docker compose exec -T server curl -fsS http://localhost:8080/api/v1/server 2>/dev/null | sed -n 's/.*"version":"\([^"]*\)".*/\1/p') at ${URL:-http://localhost:8080}"
      exit 0
    fi
    sleep 2
  done
  docker compose logs --tail 30 server
  fail "The server did not come up. The log above says why; UPGRADING.md has the known cases."
fi

# ── Questions ───────────────────────────────────────────────────────────────

step "Where things go"
say "Uploads, podcast downloads and the database stay in Docker volumes; a dump of the database"
say "is written to $DIR/backups every night."

PORT_DEFAULT="${OWN_AUDIO_PORT:-8080}"
while :; do
  ask PORT "Port for the web console and the apps" "$PORT_DEFAULT"
  case "$PORT" in ''|*[!0-9]*) say "A number, please."; continue ;; esac
  if (command -v lsof >/dev/null 2>&1 && lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1) \
     || (command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ":$PORT "); then
    say "Port $PORT is taken on this machine."; PORT_DEFAULT=$((PORT + 1)); continue
  fi
  break
done

step "Music and audiobooks you already have"
say "Folders on this machine are indexed where they are, read-only: nothing is copied or changed."
say "Music is read by its tags; audiobooks are a folder per book (Author/Title/01.mp3 …)."
say "Leave empty to skip; you can also upload from the console later."
folder() {  # folder VAR "question" default → an existing readable directory or ""
  local var="$1" question="$2" default="$3" path=""
  while :; do
    ask path "$question" "$default"
    path="${path/#\~/$HOME}"
    [ -z "$path" ] && { printf -v "$var" ''; return; }
    [ -d "$path" ] || { say "$path is not a folder."; default=""; [ "$YES" = 1 ] && { printf -v "$var" ''; return; }; continue; }
    [ -r "$path" ] || { say "$path is not readable by you."; default=""; [ "$YES" = 1 ] && { printf -v "$var" ''; return; }; continue; }
    printf -v "$var" '%s' "$(cd "$path" && pwd)"
    return
  done
}
folder MUSIC "Music folder" "${OWN_AUDIO_MUSIC:-}"
folder AUDIOBOOKS "Audiobooks folder" "${OWN_AUDIO_AUDIOBOOKS:-}"

step "The address"
LAN_IP=""
case "$(uname -s)" in
  Darwin) LAN_IP="$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || true)" ;;
  Linux) LAN_IP="$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.*src \([0-9.]*\).*/\1/p' | head -1)"
         [ -z "$LAN_IP" ] && LAN_IP="$(hostname -I 2>/dev/null | awk '{print $1}')" ;;
esac
say "Phones and the console reach the server at this address; links the server hands out use it."
say "At home, this machine's address is enough. For listening away from home, put a name with HTTPS"
say "in front later (INSTALL.md, \"Reaching it from outside\") and change PUBLIC_URL in .env."
ask URL "Address" "${OWN_AUDIO_URL:-http://${LAN_IP:-localhost}:$PORT}"
URL="${URL%/}"
case "$URL" in http://*|https://*) ;; *) URL="http://$URL" ;; esac
OTHER=""
[ -n "$LAN_IP" ] && [ "$URL" != "http://$LAN_IP:$PORT" ] && OTHER="http://$LAN_IP:$PORT"
[ "$URL" != "http://localhost:$PORT" ] && OTHER="${OTHER:+$OTHER,}http://localhost:$PORT"

# ── Summary ─────────────────────────────────────────────────────────────────

step "Ready to install"
say "  Folder       $DIR"
say "  Port         $PORT"
say "  Music        ${MUSIC:-— (upload later)}"
say "  Audiobooks   ${AUDIOBOOKS:-— (upload later)}"
say "  Address      $URL"
[ -n "$OTHER" ] && say "  Also at      $OTHER"
confirm "Install?" || { say "Nothing was changed."; exit 0; }

# ── Files ───────────────────────────────────────────────────────────────────

step "Writing the configuration"
if [ "$FROM_CHECKOUT" = 0 ]; then
  VERSION="${OWN_AUDIO_VERSION:-$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)}"
  [ -n "$VERSION" ] || fail "Could not find the newest release on GitHub. Set OWN_AUDIO_VERSION=1.0.0 (a tag of github.com/$REPO) and run again."
  curl -fsSL "$RAW/v$VERSION/docker-compose.yml" -o docker-compose.yml || fail "Could not download docker-compose.yml for $VERSION."
  mkdir -p scripts
  curl -fsSL "$RAW/v$VERSION/scripts/db-backup.sh" -o scripts/db-backup.sh || fail "Could not download scripts/db-backup.sh for $VERSION."
  curl -fsSL "$RAW/v$VERSION/.env.example" -o .env.example 2>/dev/null || true
  ok "Release $VERSION"
fi

secret() {
  if command -v openssl >/dev/null 2>&1; then openssl rand -hex 32
  else od -An -N32 -tx1 /dev/urandom | tr -d ' \n'; fi
}
umask 077
{
  echo "# Written by install.sh on $(date +%Y-%m-%d). .env.example explains every setting."
  echo "POSTGRES_PASSWORD=$(secret)"
  echo "SESSION_SECRET=$(secret)"
  echo "PORT=$PORT"
  echo "PUBLIC_URL=$URL"
  [ -n "$OTHER" ] && echo "OTHER_URLS=$OTHER"
  if [ "$(uname -s)" = Linux ]; then
    # The server reads the library folders as this user, so their permissions need no change.
    echo "PUID=$(id -u)"
    echo "PGID=$(id -g)"
  fi
  [ -n "${OWN_AUDIO_IMAGE:-}" ] && echo "SERVER_IMAGE=$OWN_AUDIO_IMAGE"
  true
} > .env
umask 022
ok ".env (two secrets made, keep this file)"

# The folders are a compose override, so a new docker-compose.yml at an upgrade never loses them.
if [ -n "$MUSIC" ] || [ -n "$AUDIOBOOKS" ]; then
  {
    echo "# Written by install.sh: your folders, mounted read-only and indexed in place."
    echo "services:"
    echo "  server:"
    echo "    environment:"
    [ -n "$MUSIC" ] && echo "      LIBRARY__MUSIC: /library/music"
    [ -n "$AUDIOBOOKS" ] && echo "      LIBRARY__AUDIOBOOKS: /library/audiobooks"
    echo "    volumes:"
    [ -n "$MUSIC" ] && echo "      - \"$MUSIC:/library/music:ro\""
    [ -n "$AUDIOBOOKS" ] && echo "      - \"$AUDIOBOOKS:/library/audiobooks:ro\""
    true
  } > docker-compose.override.yml
  ok "docker-compose.override.yml (your folders)"
fi
mkdir -p backups

# ── Start ───────────────────────────────────────────────────────────────────

step "Downloading the server (a few hundred MB, once)"
docker compose pull --quiet postgres server backup 2>&1 | grep -v -e Pulling -e Pulled || true
docker compose up -d postgres server backup 2>&1 | grep -v -e Running -e Started -e Created -e Creating -e Starting -e Healthy -e Waiting || true
printf 'Starting'
for _ in $(seq 1 60); do
  if docker compose exec -T server curl -fsS http://localhost:8080/health >/dev/null 2>&1; then
    say ""
    break
  fi
  printf '.'
  sleep 2
done
if ! docker compose exec -T server curl -fsS http://localhost:8080/health >/dev/null 2>&1; then
  say ""
  docker compose logs --tail 30 server
  fail "The server did not come up. The log above says why; INSTALL.md has the known cases."
fi
RUNNING="$(docker compose exec -T server curl -fsS http://localhost:8080/api/v1/server 2>/dev/null | sed -n 's/.*"version":"\([^"]*\)".*/\1/p')"

# ── Done ────────────────────────────────────────────────────────────────────

step "${GREEN}Your server is running${RESET}${BOLD} (own.audio ${RUNNING:-server})"
say ""
say "  1. Open ${BOLD}$URL${RESET} and create your account — the first one is the admin and founds the family."
if [ -n "$MUSIC" ] || [ -n "$AUDIOBOOKS" ]; then
  say "  2. Your folders are being read; the console shows how far it is. Nothing in them is changed."
else
  say "  2. Add music, audiobooks or podcasts from the console; folders can be added later (INSTALL.md, \"Library folders\")."
fi
say "  3. On the phone, install own.audio Music, Books or Podcasts and enter ${BOLD}$URL${RESET}."
if command -v qrencode >/dev/null 2>&1 && [ -t 1 ]; then
  say ""
  qrencode -t ANSIUTF8 -m 2 "$URL"
fi
say ""
say "${DIM}Later: run this script again to upgrade. Logs: cd $DIR && docker compose logs -f server."
say "Stop: docker compose down. Backups: $DIR/backups, nightly. Settings: .env, explained in .env.example.${RESET}"
