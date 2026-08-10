# blockly-web — the blocks-are-a-projection-of-rows showcase, for Railway.
#
# Two stages: build the single binary, ship it on slim. The runtime image
# carries NO assets — the page is compiled in (askama), and the Blockly
# editor itself loads in the *browser* from a version-pinned CDN build, per
# the arc's standing rule (JS drags puzzle pieces; Rust owns semantics).
#
# ── Why the sibling clone ───────────────────────────────────────────────
#
# This repo depends on exactly ONE external crate: `ogar-loco`, the
# substrate (call encoding, lane carvings, node layout, shared core). It is
# a PATH dependency to a sibling checkout, because a git dependency writes a
# rev pin into Cargo.lock and pins are forbidden here.
#
# Railway's build context is this repo alone, so `../OGAR` does not exist
# and `cargo` fails at manifest load — before compiling anything. The
# builder therefore clones the sibling into the layout the manifest already
# expects. OGAR is public: no token, no build secret.
#
# `--depth 1` on the default branch is deliberate — cloning a moving branch
# rather than a fixed rev is what keeps this pin-free. The trade is real and
# accepted: an image is reproducible only against OGAR's HEAD at build time.
# A deploy that needs a fixed substrate rebuilds from a fixed checkout; it
# does not get there by adding a rev here.
#
# ── Cache-busting the clone (learned the hard way) ─────────────────────
#
# "OGAR's HEAD at build time" only holds if the clone actually reruns. A
# `RUN git clone <fixed url>` line is verbatim-identical on every build, so
# Docker's (and Railway's) layer cache treats it as immutable and reuses
# whatever checkout it first produced — silently, forever, with no signal
# that it happened. A build that ran before an OGAR crate was renamed or
# removed then keeps failing (or worse, keeps *succeeding* against a stale
# substrate) on every deploy after the fix has already landed upstream, and
# nothing in the Dockerfile text changed to explain why.
#
# The `ADD <url>` below is the standard bust: BuildKit re-fetches it on every
# build and conditionally invalidates the layer (and everything after it) the
# moment OGAR's `main` tip actually moves — a real signal instead of an
# accidental one. It costs one tiny JSON response, not a second clone.
#
# Railway: binds 0.0.0.0:$PORT — PORT is injected by the platform, and 8080
# below is only the local-run fallback, never a pin.

FROM rust:1.95-bookworm AS builder
WORKDIR /build

# Cache-buster: re-fetched every build, invalidates the clone layer below
# exactly when OGAR's main tip has actually moved. See the note above.
ADD https://api.github.com/repos/AdaWorldAPI/OGAR/commits/main /tmp/ogar-head.json

# The sibling first, so an OGAR-only change does not invalidate the layer
# holding this repo's sources.
RUN git clone --depth 1 https://github.com/AdaWorldAPI/OGAR.git /build/OGAR

WORKDIR /build/blockly-rs
COPY . .
RUN cargo build --release -p blockly-web

FROM debian:bookworm-slim
RUN useradd --system --no-create-home blockly
COPY --from=builder /build/blockly-rs/target/release/blockly-web /usr/local/bin/blockly-web
USER blockly
ENV PORT=8080
EXPOSE 8080
CMD ["blockly-web"]
