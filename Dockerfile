# Portable image for VM/self-host deployment (Render uses the native rust
# runtime via render.yaml; this Dockerfile is for everywhere else).
#
# The image bundles Litestream for optional off-box backup. It stays dormant
# unless LITESTREAM_BUCKET is set at runtime — the default start path is
# unchanged. See litestream.yml and deploy/docker-entrypoint.sh.
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.97-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# workflows/ is a BUILD input, not runtime data: src/workflow.rs include_str!s
# workflows/factory-default.yaml, so the build fails without it. Anything else
# the source embeds from outside src/ has to be copied here too.
COPY workflows ./workflows
# Only generated assets enter the Rust stage; Node stays out of the runtime image.
COPY --from=web /web/dist ./web/dist
# build.rs is the third, and it fails in the least obvious way of the three: with
# no build script present cargo simply does not run one, so `OUT_DIR` is never
# set and `include!(concat!(env!("OUT_DIR"), "/assets.rs"))` in src/api/mod.rs
# fails at compile time with "environment variable `OUT_DIR` not defined" —
# which reads as a broken toolchain rather than a missing COPY. It is needed
# because the asset manifest stopped being a fixed `include_str!` list and became
# generated from whatever web/dist/assets/ holds.
COPY build.rs ./
RUN cargo build --release

# Fetch the Litestream binary (pinned release) in a throwaway stage.
FROM debian:trixie-slim AS litestream
ARG LITESTREAM_VERSION=0.3.13
ARG TARGETARCH=amd64
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends curl ca-certificates; \
    curl -fsSL -o /tmp/litestream.tar.gz \
      "https://github.com/benbjohnson/litestream/releases/download/v${LITESTREAM_VERSION}/litestream-v${LITESTREAM_VERSION}-linux-${TARGETARCH}.tar.gz"; \
    tar -C /usr/local/bin -xzf /tmp/litestream.tar.gz litestream; \
    /usr/local/bin/litestream version

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 takomo && mkdir -p /var/data && chown takomo /var/data
COPY --from=build /src/target/release/takomo /usr/local/bin/takomo
COPY --from=litestream /usr/local/bin/litestream /usr/local/bin/litestream
COPY litestream.yml /etc/litestream.yml
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
USER takomo
VOLUME /var/data
ENV TAKOMO_ALLOW_PUBLIC_BIND=1
EXPOSE 8080
# The entrypoint runs takomo directly, or under `litestream replicate` when
# LITESTREAM_BUCKET is set and the command is `serve`.
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["serve", "--bind", "0.0.0.0:8080"]
