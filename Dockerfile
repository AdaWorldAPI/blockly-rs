# blockly-web — the blocks-are-a-projection-of-rows showcase, for Railway.
#
# Two stages: build the single binary, ship it on slim. The runtime image
# carries NO assets — the page is compiled in (askama), and the Blockly
# editor itself loads in the *browser* from a version-pinned CDN build, per
# the arc's standing rule (JS drags puzzle pieces; Rust owns semantics).
#
# The build fetches the `ogar-blockly` git dependency from the public OGAR
# repository; no token is required.
#
# Railway: binds 0.0.0.0:$PORT — PORT is injected by the platform, and 8080
# below is only the local-run fallback, never a pin.

FROM rust:1.95-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p blockly-web

FROM debian:bookworm-slim
RUN useradd --system --no-create-home blockly
COPY --from=builder /src/target/release/blockly-web /usr/local/bin/blockly-web
USER blockly
ENV PORT=8080
EXPOSE 8080
CMD ["blockly-web"]
