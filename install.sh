#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────
#  audio2 — interactive installer
# ─────────────────────────────────────────────────────────
#  This script guides you through setting up audio2 with
#  Docker Compose. It generates a .env file with fresh
#  Garage storage credentials and starts the containers.
#
#  Usage:  ./install.sh
#  Re-run: safe to run again — prompts before overwriting.
# ─────────────────────────────────────────────────────────

# ── Colors ───────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

print_banner() {
    echo ""
    echo -e "${CYAN}${BOLD}"
    echo "   ╔═══════════════════════════════════════════╗"
    echo "   ║           audio2 — installer              ║"
    echo "   ║   podcasts · audiobooks · music            ║"
    echo "   ╚═══════════════════════════════════════════╝"
    echo -e "${NC}"
}

info()    { echo -e "  ${CYAN}ℹ${NC}  $*"; }
success() { echo -e "  ${GREEN}✓${NC}  $*"; }
warn()    { echo -e "  ${YELLOW}⚠${NC}  $*"; }
error()   { echo -e "  ${RED}✗${NC}  $*"; }

prepare_docker_certs() {
    local cert_dir=".docker-certs"
    mkdir -p "$cert_dir"
    touch "$cert_dir/.gitkeep"

    local copied=0
    if [ -d "/usr/local/share/ca-certificates" ]; then
        while IFS= read -r -d '' cert; do
            cp -f "$cert" "$cert_dir/$(basename "$cert")"
            copied=1
        done < <(find /usr/local/share/ca-certificates -maxdepth 1 -type f \( -name '*.crt' -o -name '*.pem' \) -print0)
    fi

    if [ "$copied" -eq 1 ]; then
        info "Copied host CA certificates into Docker build context (.docker-certs)."
    else
        info "No extra host CA certificates detected for Docker builds."
    fi
}

prompt() {
    local var_name="$1" prompt_text="$2" default="$3"
    local input
    if [ -n "$default" ]; then
        echo -ne "  ${BOLD}${prompt_text}${NC} [${default}]: "
    else
        echo -ne "  ${BOLD}${prompt_text}${NC}: "
    fi
    read -r input
    eval "$var_name=\"${input:-$default}\""
}

prompt_yes_no() {
    local var_name="$1" prompt_text="$2" default="$3"
    local input
    echo -ne "  ${BOLD}${prompt_text}${NC} [${default}]: "
    read -r input
    input="${input:-$default}"
    case "$input" in
        [Yy]|[Yy][Ee][Ss]) eval "$var_name=yes" ;;
        *) eval "$var_name=no" ;;
    esac
}

# ── Prerequisite checks ─────────────────────────────────

check_prerequisites() {
    echo ""
    echo -e "  ${BOLD}Checking prerequisites...${NC}"
    echo ""

    local ok=true

    # Docker
    if command -v docker &>/dev/null; then
        local docker_ver
        docker_ver=$(docker --version 2>/dev/null | head -1)
        success "Docker found: $docker_ver"
    else
        error "Docker not found. Install it from https://docs.docker.com/get-docker/"
        ok=false
    fi

    # Docker Compose (v2 plugin or standalone)
    if docker compose version &>/dev/null 2>&1; then
        local compose_ver
        compose_ver=$(docker compose version --short 2>/dev/null || echo "v2+")
        success "Docker Compose found: $compose_ver"
    elif command -v docker-compose &>/dev/null; then
        local compose_ver
        compose_ver=$(docker-compose --version 2>/dev/null | head -1)
        success "Docker Compose found: $compose_ver"
        warn "Consider upgrading to Docker Compose v2 (docker compose)"
    else
        error "Docker Compose not found. Install it from https://docs.docker.com/compose/install/"
        ok=false
    fi

    # Docker daemon running
    if docker info &>/dev/null 2>&1; then
        success "Docker daemon is running"
    else
        error "Docker daemon is not running. Start Docker and try again."
        ok=false
    fi

    if [ "$ok" = false ]; then
        echo ""
        error "Please fix the issues above and re-run this script."
        exit 1
    fi
    echo ""
}

# ── Configuration prompts ────────────────────────────────

collect_config() {
    echo -e "  ${BOLD}── Configuration ──────────────────────────${NC}"
    echo ""
    info "Press Enter to accept the default value shown in [brackets]."
    echo ""

    # ── Backend port ──
    prompt BACKEND_PORT "Backend port" "8080"

    # ── Database ──
    echo ""
    echo -e "  ${BOLD}Database (PostgreSQL)${NC}"
    prompt POSTGRES_USER     "  Database user"     "audio2"
    prompt POSTGRES_PASSWORD "  Database password"  "audio2"
    prompt POSTGRES_DB       "  Database name"      "audio2"
    prompt POSTGRES_PORT     "  Database port"      "5432"

    # ── Session secret ──
    echo ""
    echo -e "  ${BOLD}Security${NC}"
    DEFAULT_SECRET=$(openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 64)
    prompt AUTH_SESSION_SECRET "  JWT session secret (auto-generated)" "$DEFAULT_SECRET"

    # ── Registration ──
    prompt_yes_no REGISTRATION_OPEN "  Allow public self-registration?" "yes"

    # ── Media storage (Garage S3) ──
    echo ""
    echo -e "  ${BOLD}Media storage${NC}"
    info "audio2 stores podcasts, audiobooks, and music in Garage, an"
    info "S3-compatible object store that runs as part of the stack."
    info "Generating storage credentials..."
    GARAGE_RPC_SECRET=$(openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 64)
    # Garage requires access keys of the form "GK" + hex, secrets of 64 hex chars.
    GARAGE_ACCESS_KEY="GK$(openssl rand -hex 12 2>/dev/null || head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 24)"
    GARAGE_SECRET_KEY=$(openssl rand -hex 32 2>/dev/null || head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c 64)
    success "Generated Garage credentials"

    # ── Base URL ──
    echo ""
    prompt BASE_URL "Public URL (for browser access)" "http://localhost:${BACKEND_PORT}"

    # Browsers stream media straight from Garage, so its endpoint must be
    # reachable from the outside too — derive it from the public URL's host.
    local public_host
    public_host=$(echo "$BASE_URL" | sed -E 's#^https?://##; s#[:/].*$##')
    STORAGE_PUBLIC_ENDPOINT="http://${public_host}:3900"
    info "Media streaming endpoint: ${STORAGE_PUBLIC_ENDPOINT}"
}

# ── Generate .env file ───────────────────────────────────

generate_env() {
    local ENV_FILE=".env"

    if [ -f "$ENV_FILE" ]; then
        echo ""
        warn "A .env file already exists."
        prompt_yes_no OVERWRITE "  Overwrite it?" "no"
        if [ "$OVERWRITE" = "no" ]; then
            info "Keeping existing .env file."
            return
        fi
        cp "$ENV_FILE" "${ENV_FILE}.backup.$(date +%Y%m%d%H%M%S)"
        info "Backed up existing .env"
    fi

    local REG_VALUE="false"
    [ "$REGISTRATION_OPEN" = "yes" ] && REG_VALUE="true"

    cat > "$ENV_FILE" <<EOF
# ── audio2 configuration ────────────────────────────────
# Generated by install.sh on $(date -u +"%Y-%m-%d %H:%M UTC")
# Re-run ./install.sh to reconfigure, or edit this file directly.

# ── Database ─────────────────────────────────────────────
POSTGRES_USER=${POSTGRES_USER}
POSTGRES_PASSWORD=${POSTGRES_PASSWORD}
POSTGRES_DB=${POSTGRES_DB}
POSTGRES_PORT=${POSTGRES_PORT}

# ── Backend ──────────────────────────────────────────────
BACKEND_PORT=${BACKEND_PORT}
BASE_URL=${BASE_URL}
AUTH__SESSION_SECRET=${AUTH_SESSION_SECRET}
AUTH__REGISTRATION_OPEN=${REG_VALUE}

# ── Storage (Garage S3) ──────────────────────────────────
GARAGE_RPC_SECRET=${GARAGE_RPC_SECRET}
GARAGE_ACCESS_KEY=${GARAGE_ACCESS_KEY}
GARAGE_SECRET_KEY=${GARAGE_SECRET_KEY}
GARAGE_BUCKET=audio2
GARAGE_S3_PORT=3900
# Garage endpoint reachable from browsers (presigned media URLs point here).
STORAGE_PUBLIC_ENDPOINT=${STORAGE_PUBLIC_ENDPOINT}

# ── Logging ──────────────────────────────────────────────
RUST_LOG=audio2=info
EOF

    success "Generated .env file"
}

# ── Clean up legacy compose override ─────────────────────

generate_compose_override() {
    # Older installs bind-mounted a host audio folder; media now lives
    # in Garage's Docker volumes, so remove any stale override.
    if [ -f "docker-compose.override.yml" ]; then
        rm -f "docker-compose.override.yml"
        info "Removed legacy docker-compose.override.yml (media now stored in Garage)"
    fi
}

# ── Start containers ─────────────────────────────────────

start_services() {
    echo ""
    echo -e "  ${BOLD}── Starting audio2 ────────────────────────${NC}"
    echo ""
    info "Building and starting containers (this may take a few minutes on first run)..."
    echo ""

    if docker compose up -d --build 2>&1 | sed 's/^/    /'; then
        echo ""
        success "Containers started successfully!"
    else
        echo ""
        error "Failed to start containers. Check the output above for errors."
        info "You can retry with: docker compose up -d --build"
        exit 1
    fi

    # Wait for backend to be healthy
    echo ""
    info "Waiting for audio2 to be ready..."

    local retries=30
    local url="http://localhost:${BACKEND_PORT}/health"
    while [ $retries -gt 0 ]; do
        if curl -sf "$url" &>/dev/null; then
            success "audio2 is ready!"
            break
        fi
        retries=$((retries - 1))
        sleep 2
    done

    if [ $retries -eq 0 ]; then
        warn "audio2 did not respond within 60 seconds."
        info "Check logs with: docker compose logs -f backend"
    fi
}

# ── Print summary ────────────────────────────────────────

print_summary() {
    echo ""
    echo -e "  ${GREEN}${BOLD}══════════════════════════════════════════════${NC}"
    echo -e "  ${GREEN}${BOLD}  audio2 is installed and running!${NC}"
    echo -e "  ${GREEN}${BOLD}══════════════════════════════════════════════${NC}"
    echo ""
    echo -e "  ${BOLD}Open in your browser:${NC}"
    echo -e "    ${CYAN}${BASE_URL}${NC}"
    echo ""
    echo -e "  ${BOLD}What to do next:${NC}"

    if [ "$REGISTRATION_OPEN" = "yes" ]; then
        echo -e "    1. Register your account — the first user becomes ${BOLD}admin${NC}"
        echo -e "    2. Subscribe to a podcast, upload an audiobook, or add music"
        echo -e "    3. (Optional) Disable public registration in .env:"
        echo -e "       ${CYAN}AUTH__REGISTRATION_OPEN=false${NC}"
    else
        echo -e "    1. Enable registration in .env: ${CYAN}AUTH__REGISTRATION_OPEN=true${NC}"
        echo -e "    2. Restart: ${CYAN}docker compose up -d${NC}"
        echo -e "    3. Register your admin account, then disable registration again"
    fi

    echo ""
    echo -e "  ${BOLD}Useful commands:${NC}"
    echo -e "    ${CYAN}docker compose logs -f backend${NC}   — view live logs"
    echo -e "    ${CYAN}docker compose stop${NC}              — stop (keep data)"
    echo -e "    ${CYAN}docker compose start${NC}             — restart"
    echo -e "    ${CYAN}docker compose down${NC}              — remove containers"
    echo -e "    ${CYAN}./install.sh${NC}                     — reconfigure"
    echo ""

    echo -e "  ${BOLD}Media storage:${NC} Garage S3 (Docker volumes garage_meta, garage_data)"
    echo -e "  ${BOLD}Database:${NC}      Docker volume (postgres_data)"
    echo -e "  ${BOLD}Config:${NC}        .env"
    echo ""
    echo -e "  ${BOLD}Documentation:${NC} ${CYAN}INSTALL.md${NC}  ·  ${CYAN}README.md${NC}"
    echo ""
}

# ── Main ─────────────────────────────────────────────────

main() {
    # cd to the script's directory (repo root)
    cd "$(dirname "$0")"

    print_banner
    check_prerequisites
    collect_config
    generate_env
    generate_compose_override
    prepare_docker_certs
    start_services
    print_summary
}

main "$@"
