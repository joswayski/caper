FROM rust:1.94-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY apps/sfu/Cargo.toml apps/sfu/Cargo.toml
COPY apps/sfu/src apps/sfu/src
# The API is a separate image; its workspace manifest only needs to resolve.
COPY apps/api/Cargo.toml apps/api/Cargo.toml
RUN mkdir -p apps/api/src && touch apps/api/src/lib.rs && echo "fn main() {}" > apps/api/src/main.rs

RUN cargo build --locked --release --package caper-sfu

FROM debian:bookworm-slim AS sfu

RUN groupadd --gid 10001 caper \
    && useradd --uid 10001 --gid caper --no-create-home --shell /usr/sbin/nologin caper

COPY --from=builder /build/target/release/caper-sfu /usr/local/bin/caper-sfu
# Provider benchmark (docs/media.md#benchmark-cloudflare-against-the-sfu).
COPY --from=builder /build/target/release/caper-sfu-bench /usr/local/bin/caper-sfu-bench

USER 10001:10001
EXPOSE 8080 50000/udp
ENTRYPOINT ["/usr/local/bin/caper-sfu"]
