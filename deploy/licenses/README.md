# Bundled renderer provenance

Takomo's Dockerfile redistributes components from the immutable Kroki 0.32.1
images pinned there. The image carries these notices in
`/usr/local/share/doc/takomo-renderers`.

- Kroki and its Mermaid companion: MIT, source and build recipes at
  https://github.com/yuzutech/kroki/tree/v0.32.1 . The Mermaid companion's
  `server.listen(8002)` is changed to `server.listen(8002, '127.0.0.1')`; production
  dependencies are installed from its original lockfile on Debian. Generated
  Mermaid browser assets are copied unchanged from the upstream image.
- PlantUML native 1.2026.6 (commit 6287b33): GPLv3, source at
  https://github.com/plantuml/plantuml/tree/v1.2026.6 and source archive at
  https://github.com/plantuml/plantuml/archive/refs/tags/v1.2026.6.tar.gz . The
  native executable and its GraalVM AWT support libraries are copied unchanged
  from Kroki. See the above Kroki tag's Dockerfile for the native build provenance.
- D2 0.7.1: MPL 2.0, source at https://github.com/terrastruct/d2/tree/v0.7.1
  and source archive https://github.com/terrastruct/d2/archive/refs/tags/v0.7.1.tar.gz .
- Node: binary from the official `node:24-trixie-slim` build stage; its complete
  upstream license file is copied alongside these notices. Source and release
  archives: https://nodejs.org/dist/ .
- Debian-packaged Java, Chromium, Graphviz and fonts retain their package
  copyright notices under `/usr/share/doc`. Debian source packages are available
  through https://sources.debian.org/ . npm packages retain their distributed
  metadata/notices under `/opt/kroki/mermaid/node_modules`.

When redistributing images, retain these notices and provide the corresponding
source required by each component license. Update versions, source references and
notices together with Dockerfile pins; do not assume a new tag has the same terms.
