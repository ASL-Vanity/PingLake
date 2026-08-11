FROM node:22-bookworm-slim@sha256:a17d50af28002a160548bd4225b3cfcb12c5efcb171f79e68758f2885fb1b066 AS web-builder
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.97-slim-bookworm@sha256:8d7cf6bc81180ec559dca5dd149d9ce78c9cfa270da33ab5690c6b9a5190d953 AS rust-builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY --from=web-builder /src/web/dist ./web/dist
RUN cargo build --release --locked -p pinglake-hub

FROM debian:bookworm-slim@sha256:362e64223cc0da95422b3b13c045186fc0a81250e765d31c025fbddf257f6143
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data --shell /usr/sbin/nologin pinglake \
    && mkdir -p /data \
    && chown pinglake:pinglake /data
COPY --from=rust-builder /src/target/release/pinglake-hub /usr/local/bin/pinglake-hub
USER pinglake
ENV PINGLAKE_BIND=0.0.0.0:8090
ENV PINGLAKE_DATABASE=/data/pinglake.db
EXPOSE 8090
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/pinglake-hub"]
