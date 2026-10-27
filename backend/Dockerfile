# syntax=docker/dockerfile:1
# FFmpeg (with Chromaprint) and aubio are compiled here instead of coming from
# Debian: Debian's ffmpeg pulls in LLVM, mesa, flite, librsvg and z3, and its
# aubio a Python with NumPy, together ~500 MB of a ~920 MB image. Trivy's scan
# of the Debian packages cannot see these builds, so their security updates
# come from bumping the versions below (and the checksums with them) — see
# CLAUDE.md §8. The stage depends on nothing else, so CI's layer cache keeps it
# until one of these lines changes.
ARG FFMPEG_VERSION=8.1.3
ARG FFMPEG_SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
ARG CHROMAPRINT_VERSION=1.6.2
ARG CHROMAPRINT_SHA256=d54ada62c16b2db396d3251ec54fc7a58909c163131a1cfcf7fa6407dbc14360
ARG AUBIO_VERSION=0.4.9
ARG AUBIO_SHA256=d48282ae4dab83b3dc94c16cf011bcb63835c1c02b515490e1883049c3d1f3da

# ── Stage 0: ffmpeg, ffprobe and aubiotrack ──────────────
# The same Debian as the runtime, so the binaries match its glibc and libs.
FROM debian:bookworm-slim AS media-tools
ARG FFMPEG_VERSION FFMPEG_SHA256 CHROMAPRINT_VERSION CHROMAPRINT_SHA256 AUBIO_VERSION AUBIO_SHA256
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl xz-utils bzip2 build-essential pkg-config cmake nasm \
    python3 python-is-python3 libmp3lame-dev libssl-dev zlib1g-dev libbz2-dev \
    && rm -rf /var/lib/apt/lists/*
COPY .docker-certs/ /tmp/audio2-extra-certs/
RUN if find /tmp/audio2-extra-certs -type f \( -name '*.crt' -o -name '*.pem' \) | grep -q .; then \
      cp /tmp/audio2-extra-certs/* /usr/local/share/ca-certificates/ && \
      update-ca-certificates; \
    fi
WORKDIR /src
RUN curl -fsSL -o ffmpeg.tar.xz "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" \
 && echo "${FFMPEG_SHA256}  ffmpeg.tar.xz" | sha256sum -c - \
 && curl -fsSL -o chromaprint.tar.gz "https://github.com/acoustid/chromaprint/releases/download/v${CHROMAPRINT_VERSION}/chromaprint-${CHROMAPRINT_VERSION}.tar.gz" \
 && echo "${CHROMAPRINT_SHA256}  chromaprint.tar.gz" | sha256sum -c - \
 && curl -fsSL -o aubio.tar.bz2 "https://aubio.org/pub/aubio-${AUBIO_VERSION}.tar.bz2" \
 && echo "${AUBIO_SHA256}  aubio.tar.bz2" | sha256sum -c - \
 && mkdir ffmpeg chromaprint aubio \
 && tar -xJf ffmpeg.tar.xz --strip-components=1 -C ffmpeg \
 && tar -xzf chromaprint.tar.gz --strip-components=1 -C chromaprint \
 && tar -xjf aubio.tar.bz2 --strip-components=1 -C aubio
# Chromaprint with its bundled KissFFT: built against FFmpeg's own FFT it would
# need an FFmpeg before FFmpeg. Static, so only ffmpeg carries it.
RUN cmake -S chromaprint -B chromaprint/build -DCMAKE_BUILD_TYPE=Release \
      -DBUILD_SHARED_LIBS=OFF -DBUILD_TOOLS=OFF -DBUILD_TESTS=OFF -DFFT_LIB=kissfft \
      -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DCMAKE_INSTALL_PREFIX=/opt/chromaprint \
 && cmake --build chromaprint/build -j"$(nproc)" && cmake --install chromaprint/build
# Every native decoder, demuxer, muxer, parser, filter and encoder stays: limiting
# encoders and filters to audio saved 3 MB of 21 and risked a code path that
# meets a cover picture. External libraries are only LAME (MP3 out),
# Chromaprint (`-f chromaprint`) and OpenSSL (https links into object storage);
# the one device is `-f lavfi` (generated tones, as the tests make them).
# Shared libraries, because ffmpeg and ffprobe would each carry a static copy.
RUN cd ffmpeg \
 && PKG_CONFIG_PATH=/opt/chromaprint/lib/pkgconfig ./configure --prefix=/usr/local \
      --enable-shared --disable-static --disable-debug --disable-doc --disable-ffplay \
      --disable-indevs --enable-indev=lavfi --disable-outdevs --disable-autodetect --enable-pthreads \
      --enable-zlib --enable-bzlib --enable-iconv --enable-openssl --enable-libmp3lame --enable-chromaprint \
      --extra-libs="-lstdc++ -lm" \
 && make -j"$(nproc)" && make install DESTDIR=/out \
 && rm -rf /out/usr/local/include /out/usr/local/lib/pkgconfig /out/usr/local/share
# aubio reads WAV only (no libav, sndfile or samplerate): ffmpeg decodes for it
# (music/analysis.rs). `aubio` itself is a Python program, so the C beat
# tracker `aubiotrack` is what ships. Its waf predates Python 3.11's open().
RUN cd aubio \
 && sed -i "s/'rU'/'r'/" waflib/ConfigSet.py waflib/Context.py \
 && ./waf configure --prefix=/usr/local --build-type=release \
      --disable-fftw3f --disable-fftw3 --disable-intelipp --disable-jack --disable-sndfile \
      --disable-avcodec --disable-samplerate --disable-accelerate --disable-apple-audio \
      --disable-blas --disable-atlas --disable-docs --disable-tests \
 && ./waf build -j"$(nproc)" --targets=aubiotrack \
 && cp build/examples/aubiotrack /out/usr/local/bin/ \
 && cp build/src/libaubio.so /out/usr/local/lib/libaubio.so.5 \
 && strip --strip-unneeded /out/usr/local/bin/* /out/usr/local/lib/*.so.*

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
# the image scan in CI refuses fixed CRITICAL/HIGH findings. CI passes the
# day as APT_DAY: a cached layer is reused for a day at most, or the upgrade
# would never run again (libexpat stayed at a fixed CVE for weeks that way).
ARG APT_DAY=unset
RUN echo "packages of $APT_DAY" && apt-get update && apt-get upgrade -y --no-install-recommends && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libmp3lame0 libssl3 libstdc++6 zlib1g libbz2-1.0 \
    && rm -rf /var/lib/apt/lists/*

# ffmpeg, ffprobe and aubiotrack from the media-tools stage, with the Debian
# libraries above as their only dependencies; the build stops here if one is missing.
COPY --from=media-tools /out/usr/local/ /usr/local/
RUN ldconfig && ! ldd /usr/local/bin/* | grep 'not found' && ffmpeg -hide_banner -version | head -1

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

