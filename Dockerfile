# Portable image for VM/self-host deployment (Render uses the native rust
# runtime via render.yaml; this Dockerfile is for everywhere else).
#
# The image bundles Litestream for optional off-box backup. It stays dormant
# unless LITESTREAM_BUCKET is set at runtime — the default start path is
# unchanged. See litestream.yml and deploy/docker-entrypoint.sh.
FROM rust:1.97-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
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
