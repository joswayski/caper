# Media worker: AWS Lambda container image (custom runtime, linux/arm64).
# Build from the repository root:
#   docker buildx build --platform linux/arm64 -f apps/media-worker/Dockerfile -t caper-media-worker .
FROM rust:1.94-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
# Cargo loads every workspace member's manifest; only the worker is built.
COPY apps/api/Cargo.toml apps/api/Cargo.toml
COPY apps/media-worker/Cargo.toml apps/media-worker/Cargo.toml
COPY apps/media-worker/src apps/media-worker/src

RUN cargo build --locked --release --package caper-media-worker --bin caper-media-worker

FROM debian:trixie-slim AS worker

# ffmpeg/ffprobe (video, audio, GIF/APNG), libvips with HEIF/AVIF decoding
# (the libheif plugins are only recommended, so name them), cwebp, avifenc
# and jpegtran. No recommends, docs or man pages.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        ffmpeg \
        libavif-bin \
        libheif-plugin-dav1d \
        libheif-plugin-libde265 \
        libjpeg-turbo-progs \
        libvips-tools \
        webp \
    && rm -rf /var/lib/apt/lists/* /usr/share/doc /usr/share/man /usr/share/info \
    && groupadd --gid 10001 caper \
    && useradd --uid 10001 --gid caper --no-create-home --shell /usr/sbin/nologin caper

COPY --from=builder /build/target/release/caper-media-worker /usr/local/bin/caper-media-worker

# Lambda only allows writes under /tmp.
ENV HOME=/tmp
WORKDIR /tmp
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/caper-media-worker"]
