# Portable image for VM/self-host deployment (Render uses the native rust
# runtime via render.yaml; this Dockerfile is for everywhere else).
#
# The default image includes private diagram renderers and optional Litestream
# backup. See docs/hosting.md and docs/diagrams.md for runtime behavior.
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
# Only generated frontend assets enter the Rust build stage.
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
ARG TARGETARCH
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends curl ca-certificates; \
    curl -fsSL -o /tmp/litestream.tar.gz \
      "https://github.com/benbjohnson/litestream/releases/download/v${LITESTREAM_VERSION}/litestream-v${LITESTREAM_VERSION}-linux-${TARGETARCH}.tar.gz"; \
    tar -C /usr/local/bin -xzf /tmp/litestream.tar.gz litestream; \
    /usr/local/bin/litestream version

# Upstream pins match the optional external-service Compose deployment.
FROM yuzutech/kroki:0.32.1@sha256:6980bfb218b48b74ea14b888d9c7e8c032d1cb6325f3292277abdf62483abd9d AS kroki
FROM yuzutech/kroki-mermaid:0.32.1@sha256:ad6721646aabfb8b5f5005db2daa6e37189c25e18aab19a8ef8590b6171cd192 AS mermaid

# Never copy Alpine's Node, Chromium or native npm modules into Debian.
# Reinstall the companion's locked production dependencies against glibc instead.
FROM node:24-trixie-slim AS mermaid-build
WORKDIR /opt/kroki/mermaid
COPY --from=mermaid /usr/local/kroki/package*.json ./
COPY --from=mermaid /usr/local/lib/browser-instance /opt/kroki/lib/browser-instance
RUN PUPPETEER_SKIP_DOWNLOAD=true npm ci --omit=dev --ignore-scripts
COPY --from=mermaid /usr/local/kroki/src ./src
COPY --from=mermaid /usr/local/kroki/assets ./assets
# Upstream has no listening-host option. Assert the pinned source before adapting
# its one listen call so publishing an extra Docker port cannot expose it.
RUN node --input-type=module -e "import fs from 'node:fs'; const p='src/index.js'; const s=fs.readFileSync(p,'utf8'); if(s.split('server.listen(8002)').length!==2) throw Error('upstream listen changed'); fs.writeFileSync(p,s.replace('server.listen(8002)', \"server.listen(8002, '127.0.0.1')\"));"

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates openjdk-21-jre-headless \
       chromium graphviz fonts-dejavu-core fonts-liberation python3 tini util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 takomo \
    && useradd -r -u 10002 -m -d /var/lib/kroki kroki \
    && useradd -r -u 10003 -m -d /var/lib/mermaid mermaid \
    && install -d -m 0700 -o takomo -g takomo /var/data \
    && chmod 0700 /var/lib/kroki /var/lib/mermaid
COPY --from=build /src/target/release/takomo /usr/local/bin/takomo
COPY --from=litestream /usr/local/bin/litestream /usr/local/bin/litestream
COPY --from=mermaid-build /usr/local/bin/node /usr/local/bin/node
COPY --from=mermaid-build /opt/kroki /opt/kroki
COPY --from=kroki /usr/local/kroki/kroki-server.jar /opt/kroki/kroki-server.jar
# Kroki's PlantUML native image needs its matching GraalVM AWT sidecar libraries.
# Keep them beside the executable, separate from Debian's JVM. D2 is static.
# Debian trixie supplies a newer compatible glibc than upstream Ubuntu 24.04.
COPY --from=kroki /usr/bin/plantuml /usr/bin/lib*.so /opt/plantuml/
COPY --from=kroki /usr/bin/d2 /usr/bin/d2
COPY --from=kroki /etc/kroki/logback.xml /etc/kroki/logback.xml
RUN /opt/plantuml/plantuml -version && d2 --version && node --version
COPY deploy/licenses /usr/local/share/doc/takomo-renderers
COPY --from=mermaid-build /usr/local/LICENSE /usr/local/share/doc/takomo-renderers/Node-LICENSE
COPY litestream.yml /etc/litestream.yml
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY deploy/container-supervisor.py deploy/container-healthcheck.py /usr/local/lib/takomo/
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
VOLUME /var/data
ENV TAKOMO_ALLOW_PUBLIC_BIND=1
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=15s --start-period=60s --retries=3 \
    CMD ["python3", "/usr/local/lib/takomo/container-healthcheck.py"]
# Tini reaps adopted descendants; the supervisor forwards signals to every
# service process group and exits the container when any main service fails.
ENTRYPOINT ["/usr/bin/tini", "--", "python3", "/usr/local/lib/takomo/container-supervisor.py"]
CMD ["serve", "--bind", "0.0.0.0:8080"]
