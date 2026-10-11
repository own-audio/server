# syntax=docker/dockerfile:1
# ── Stage 1: frontend builder ────────────────────────────
FROM node:22-slim AS frontend-builder

COPY .docker-certs/ /tmp/audio2-extra-certs/
RUN if find /tmp/audio2-extra-certs -type f \( -name '*.crt' -o -name '*.pem' \) | grep -q .; then \
            apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
            cp /tmp/audio2-extra-certs/* /usr/local/share/ca-certificates/ && \
            update-ca-certificates && \
            rm -rf /var/lib/apt/lists/*; \
        fi

WORKDIR /build/frontend

COPY frontend/package.json frontend/package-lock.json* ./
RUN --mount=type=cache,target=/root/.npm npm ci --prefer-offline

COPY frontend/ ./
# Build to an explicit absolute output dir (bypasses vite.config outDir pointing at backend/)
RUN npx vite build --outDir /build/ui-dist --emptyOutDir

# ── Stage 2: backend builder ──────────────────────────────
FROM rust:1.97-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY .docker-certs/ /tmp/audio2-extra-certs/
RUN if find /tmp/audio2-extra-certs -type f \( -name '*.crt' -o -name '*.pem' \) | grep -q .; then \
      cp /tmp/audio2-extra-certs/* /usr/local/share/ca-certificates/ && \
      update-ca-certificates; \
    fi

ARG BIN=audio2

WORKDIR /build

# The production runner builds this on the same 4 GB host that serves the
# app. Cargo defaults to one rustc per core (8), and the heavier crates peak
# near 1.5 GB each — the kernel OOM-killed rustc twice on 2026-08-27. Two at
# a time was meant to fit alongside the running containers, but on
# 2026-09-26 the two parallel LTO codegen processes alone used ~2.75 GB and
# triggered a box-wide OOM that killed unrelated containers (mindmapvault's
# postgres). The runner's job container is now capped at 2.5 GB
# (audio2-runner/data/config.yml on the VPS); one job at a time (~1.65 GB
# peak) fits that cap with room to spare, two did not.
ENV CARGO_BUILD_JOBS=1
# Incremental release builds reuse the cached `target/`: a change recompiles the parts of our
# own crate it touches instead of all of it, which alone took minutes. The binary is a little
# less optimised; nothing here is CPU-bound enough to notice.
ENV CARGO_INCREMENTAL=1

# The registry and `target/` live in BuildKit caches that outlast a build, so a change to the
# code — or to Cargo.toml, which every release's version bump touches — recompiles only what
# changed. The old dummy-`main.rs` layer was thrown away by each version bump, rebuilding
# every dependency in release mode on every deploy.
COPY Cargo.toml Cargo.lock* ./
COPY backend ./backend
# COPY keeps the files' times; with target/ cached between builds, a file
# edited before the previous build finished looked older than that build and
# was skipped. Touching our own sources rebuilds this crate only.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    find backend/src -type f -exec touch {} + && \
    cargo build --release -p audio2 && cp target/release/audio2 /build/audio2

# ── Stage 3: runtime ─────────────────────────────────────
FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="own.audio server" \
      org.opencontainers.image.description="Self-hosted server for a family's audiobooks, podcasts and music. AGPL-3.0." \
      org.opencontainers.image.source="https://github.com/own-audio/server" \
      org.opencontainers.image.url="https://demo.own.audio" \
      org.opencontainers.image.documentation="https://github.com/own-audio/server/blob/main/INSTALL.md" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

# `upgrade` first: the base tag lags Debian's security updates by days, and
# the image scan in CI refuses fixed CRITICAL/HIGH findings (yt-dlp pulls in
# python, which gets them regularly).
RUN apt-get update && apt-get upgrade -y --no-install-recommends && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    yt-dlp \
    ffmpeg \
    aubio-tools \
    && rm -rf /var/lib/apt/lists/*

COPY .docker-certs/ /tmp/audio2-extra-certs/
RUN if find /tmp/audio2-extra-certs -type f \( -name '*.crt' -o -name '*.pem' \) | grep -q .; then \
            cp /tmp/audio2-extra-certs/* /usr/local/share/ca-certificates/ && \
            update-ca-certificates; \
        fi

# glibc keeps freed memory per thread arena and grows its mmap threshold after
# the first large buffer, so the process holds the peak of every burst; these
# hand it back (docs/RAM_USAGE.md: test-suite peak 272 MiB -> 28 MiB, same
# speed). Override in compose if a deployment measures otherwise.
ENV MALLOC_ARENA_MAX=2 \
    MALLOC_MMAP_THRESHOLD_=131072 \
    MALLOC_TRIM_THRESHOLD_=131072

# Local storage's default folder (STORAGE__KIND=local); a volume mounts here.
RUN mkdir -p /data/media

WORKDIR /app

COPY --from=builder       /build/audio2                         /app/audio2
COPY --from=builder       /build/backend/migrations             /app/migrations
COPY --from=frontend-builder /build/ui-dist                     /app/ui/dist

EXPOSE 8080

# Started as root only to hand /data/media to PUID:PGID, then dropped to them.
COPY backend/docker-entrypoint.sh /app/docker-entrypoint.sh
ENTRYPOINT ["/app/docker-entrypoint.sh"]

