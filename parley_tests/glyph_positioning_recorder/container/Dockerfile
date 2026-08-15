# Combined recorder image: pinned chrome-headless-shell + chromedriver, plus
# Skia's own `skp_parser` built at the exact Skia revision Chromium bundles.
#
# One image (not two), built entirely for linux/amd64 even on arm64 hosts —
# see doc/glyph-positioning-chrome-parity.md's Phase 3 discussion for why
# (matching CI's native arch beats avoiding local emulation).
#
# Pin provenance (re-derive after a version bump — see ../README.md):
#   Chrome for Testing Stable 151.0.7922.77
#   -> https://chromium.googlesource.com/chromium/src/+/151.0.7922.77/DEPS
#      'skia_revision': 'f4eee5c6735d33a829ab2cfbf3fe23d0376e7992'
#
# Neither Chrome for Testing artifact is checksummed by Google (no sha256 in
# their manifest, no separate SHASUMS file) — verified during Phase 3 grilling.
# We rely on plain HTTPS + `curl -f` + `unzip` failing loudly on a truncated
# download, deliberately not self-pinning a hash we'd compute ourselves.

# ---- stage 1: build skp_parser at the DEPS-matched Skia revision ----------
# trixie (not bookworm) for clang-19: bookworm's clang-14 cannot compile
# libstdc++-12's C++20 <ranges> (src/pdf/SkPDFTag.cpp fails).
FROM --platform=linux/amd64 debian:trixie-slim AS skia-builder

ARG SKIA_REV=f4eee5c6735d33a829ab2cfbf3fe23d0376e7992

RUN apt-get update && apt-get install -y --no-install-recommends \
      git python3 curl unzip ca-certificates \
      clang ninja-build \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /skia
RUN git init -q \
    && git remote add origin https://skia.googlesource.com/skia.git \
    && git fetch --depth 1 -q origin ${SKIA_REV} \
    && git checkout -q FETCH_HEAD

# Fetches the third_party deps recorded in Skia's own DEPS (needs network;
# this is image-build time only). `git-sync-deps` has no allowlist/denylist
# for individual DEPS entries — it
# unconditionally clones everything in the `deps` dict, which we can't avoid
# without patching Skia's own script (the doc's "unmodified tooling"
# constraint). GIT_SYNC_DEPS_SKIP_EMSDK is the one thing it *does* let us
# skip: activate-emsdk downloads and unpacks a full Node.js + Emscripten
# toolchain for Skia's WASM/CanvasKit targets, which `skp_parser` (a native
# GN target) never touches.
ENV GIT_SYNC_DEPS_SKIP_EMSDK=1
# git-sync-deps fires off every DEPS entry as a concurrent, unthrottled git
# clone/fetch — reliably triggers HTTP 429 from googlesource.com's git
# servers (seen consistently, not a one-off blip). It has no built-in
# retry/backoff, and each repo's checkout is already incremental (a re-run
# just does `git checkout` against an existing clone unless that repo
# specifically failed), so retry the whole invocation externally rather than
# patching the script.
RUN for i in 1 2 3 4 5 6; do \
      python3 tools/git-sync-deps && exit 0; \
      echo "git-sync-deps attempt ${i} failed (likely rate-limited); retrying..." >&2; \
      sleep 20; \
    done; \
    echo "git-sync-deps failed after 6 attempts" >&2; exit 1

RUN python3 bin/fetch-gn

# Minimal config: no GPU backends, no system libs, no fontconfig. We only need
# SKP deserialisation + the debugger's draw-command JSON dump.
RUN ./bin/gn gen out/Release --args=' \
      is_debug=false \
      cc="clang" cxx="clang++" \
      skia_use_gl=false \
      skia_use_egl=false \
      skia_use_vulkan=false \
      skia_use_x11=false \
      skia_use_fontconfig=false \
      skia_enable_pdf=false \
      skia_use_system_expat=false \
      skia_use_system_freetype2=false \
      skia_use_system_harfbuzz=false \
      skia_use_system_icu=false \
      skia_use_system_libjpeg_turbo=false \
      skia_use_system_libpng=false \
      skia_use_system_libwebp=false \
      skia_use_system_zlib=false \
    '

RUN ninja -C out/Release skp_parser

# ---- stage 2: runtime — chrome-headless-shell + chromedriver + skp_parser -
FROM --platform=linux/amd64 debian:trixie-slim

ARG CFT_VERSION=151.0.7922.77
ARG BASE=https://storage.googleapis.com/chrome-for-testing-public
# Pinned static Deno binary for the in-container HTTP agent (see
# `container/agent/` and `doc/glyph-positioning-recorder-agent.md`). This is the
# single source of truth for the version: the CI job that lints/typechecks the agent
# reads it back out of this file rather than pinning it a second time.
ARG DENO_VERSION=2.9.5

# `iproute2` is gone: it existed only for the old entrypoint's `ip route` gateway
# autodetection, which the agent-based redesign deleted entirely (see
# `../entrypoint.sh` and `doc/glyph-positioning-recorder-agent.md`).
RUN apt-get update && apt-get install -y --no-install-recommends \
      curl unzip ca-certificates \
      fonts-liberation libasound2t64 libatk-bridge2.0-0 libatk1.0-0 libatspi2.0-0 \
      libcairo2 libcups2 libdbus-1-3 libdrm2 libgbm1 libglib2.0-0 libnspr4 \
      libnss3 libpango-1.0-0 libpangocairo-1.0-0 libx11-6 libxcb1 libxcomposite1 \
      libxdamage1 libxext6 libxfixes3 libxkbcommon0 libxrandr2 xdg-utils \
    && rm -rf /var/lib/apt/lists/*

# Same pattern as the Chrome for Testing downloads below: plain HTTPS + `curl -f` +
# `unzip` failing loudly on a truncated download, no self-pinned hash.
RUN curl -fsSL "https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-x86_64-unknown-linux-gnu.zip" -o /tmp/deno.zip \
    && unzip -q /tmp/deno.zip -d /usr/local/bin \
    && rm /tmp/deno.zip \
    && chmod +x /usr/local/bin/deno

# Only chrome-headless-shell + chromedriver are shipped — the full `chrome`
# binary is unused in production (it was only ever a manual debugging
# fallback in the Phase 0 spike's driver) and would just add image bloat.
WORKDIR /opt
RUN curl -fsSL "${BASE}/${CFT_VERSION}/linux64/chrome-headless-shell-linux64.zip" -o hs.zip \
    && unzip -q hs.zip && rm hs.zip \
    && curl -fsSL "${BASE}/${CFT_VERSION}/linux64/chromedriver-linux64.zip" -o cd.zip \
    && unzip -q cd.zip && rm cd.zip \
    && chmod +x /opt/chrome-headless-shell-linux64/chrome-headless-shell \
                /opt/chromedriver-linux64/chromedriver

COPY --from=skia-builder /skia/out/Release/skp_parser /usr/local/bin/skp_parser

# Run as non-root: --no-sandbox already strips one isolation layer from
# Chrome itself, so avoid stacking "also root" on top of that. Note this
# container is NOT network-isolated (see ../README.md) — the realistic
# threat model is low regardless, since inputs are self-generated test HTML,
# not adversarial.
RUN groupadd -r parley && useradd -r -g parley -m -d /home/parley parley \
    && mkdir -p /skp && chown parley:parley /skp

COPY --chmod=755 entrypoint.sh /entrypoint.sh

USER parley
WORKDIR /home/parley

# `deno bundle` (used by the entrypoint to bundle `harness.ts` at container startup —
# see `../entrypoint.sh`) fetches its esbuild binary from the npm registry on first
# use. Warming that into the image now, against a throwaway file, means the running
# container never needs a network round-trip for it — the cache keys off the esbuild
# version, not the project being bundled, so this warms real startup's cache too.
RUN echo 'export {};' > /tmp/warm.ts \
    && deno bundle --platform=browser --format=iife -o /tmp/warm.bundle.js /tmp/warm.ts \
    && rm -f /tmp/warm.ts /tmp/warm.bundle.js

EXPOSE 9515 9516
ENTRYPOINT ["/entrypoint.sh"]
