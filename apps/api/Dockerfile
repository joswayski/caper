FROM rust:1.94-bookworm AS builder

WORKDIR /build

# Cargo must be able to load every workspace member even though only caper-api is built.
# Stub the unused desktop crate so API image CI does not depend on Tauri sources.
COPY Cargo.toml Cargo.lock ./
COPY apps/api/Cargo.toml apps/api/Cargo.toml
COPY apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/Cargo.toml
RUN mkdir -p apps/desktop/src-tauri/src \
    && printf 'fn main() {}\n' > apps/desktop/src-tauri/build.rs \
    && printf 'fn main() {}\n' > apps/desktop/src-tauri/src/main.rs \
    && printf '\n' > apps/desktop/src-tauri/src/lib.rs
COPY apps/api/src apps/api/src
COPY apps/api/build.rs apps/api/build.rs
RUN mkdir -p apps/api/migrations

RUN cargo build --locked --release --package caper-api

FROM debian:bookworm-slim AS api

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 caper \
    && useradd --uid 10001 --gid caper --no-create-home --shell /usr/sbin/nologin caper

COPY --from=builder /build/target/release/caper-api /usr/local/bin/caper-api

USER 10001:10001
EXPOSE 3001
ENTRYPOINT ["/usr/local/bin/caper-api"]
