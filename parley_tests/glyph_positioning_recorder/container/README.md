# Recorder container

The production image for the glyph-positioning Chrome-parity harness: pinned
`chrome-headless-shell` + `chromedriver` (Chrome for Testing), Skia's own
`skp_parser` built at the exact Skia revision that Chromium build bundles, and a
pinned Deno binary that runs the in-container HTTP agent (`agent/`). One combined
multi-stage image, always `linux/amd64` (Chrome for Testing has no linux-arm64
build), even when built on an arm64 host.

See [`../../../doc/glyph-positioning-recorder-agent.md`](../../../doc/glyph-positioning-recorder-agent.md)
for the agent's architecture (the endpoint table, why its one mount is exempt from an
earlier bind-mount staleness finding, the launcher-as-lockstep-contract) and
[`../../../doc/glyph-positioning-chrome-parity.md`](../../../doc/glyph-positioning-chrome-parity.md)
for the harness's design history.

## Version-pinning bump procedure

Three identifiers must move together; there is no single manifest with all three:

```sh
# 1. Pick a Chrome for Testing version (Stable channel):
curl -s https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json \
  | grep -A3 '"channel": "Stable"'

# 2. Find the Skia revision that exact Chromium tag bundles:
curl -s "https://chromium.googlesource.com/chromium/src/+/<VERSION>/DEPS?format=TEXT" \
  | base64 -d | grep skia_revision

# 3. Pick a Deno release (independent of the above — Deno only runs the agent):
curl -s https://api.github.com/repos/denoland/deno/releases/latest | grep tag_name
```

Update `ARG CFT_VERSION`, `ARG SKIA_REV` and `ARG DENO_VERSION` in `Dockerfile` (and
the comment block at its top, which records the Chrome/Skia pin's provenance).
`ARG DENO_VERSION` is the *only* place the Deno version is pinned — the CI job that
lints/typechecks `agent/` reads it back out of this file rather than pinning it a
second time.

Neither Chrome for Testing artifact (`chrome-headless-shell`, `chromedriver`) is
checksummed by Google — their `known-good-versions-with-downloads.json` manifest has
only `platform`/`url` per download, no `SHASUMS`-style file. Deno's GitHub releases do
publish `.sha256sum` files, but the Dockerfile deliberately does not verify against
them, for the same reason as the Chrome downloads: plain HTTPS plus `curl -f`/`unzip`
failing loudly on a truncated download, rather than self-pinning a hash computed from
the same download it would be checked against.

## Building

```sh
docker build -t parley-glyph-recorder:latest .
```

Both stages target `linux/amd64` explicitly. On an arm64 host (e.g. Apple Silicon) the
`skia-builder` stage runs under QEMU emulation — expect this to be considerably slower
than the ~4m46s native build the project's Phase 0 spike measured. BuildKit
layer-caches this stage across rebuilds as long as `ARG SKIA_REV` and the `RUN` steps
above it don't change, so this cost is paid once per pin bump, not per build — a
runtime-stage-only change (e.g. bumping `ARG DENO_VERSION`, editing `entrypoint.sh`)
rebuilds in seconds.

## Running

`container/run.sh` is the normal way to run this: it builds the image if needed,
restarts the container, waits for both ports, then execs the requested recorder
binary.

```sh
container/run.sh fuzz_loop --jobs 4 --max-cases 300
container/run.sh regenerate_goldens
container/run.sh minimise --jobs 4
```

Under the hood, that script runs (roughly):

```sh
docker rm -f parley-recorder
docker run -d --name parley-recorder \
  -v "$PWD/agent:/agent:ro" \
  -p 127.0.0.1:9515:9515 \
  -p 127.0.0.1:9516:9516 \
  parley-glyph-recorder:latest
```

There is exactly **one** mount: `agent/` (this directory), read-only, at `/agent`. It
holds the in-container HTTP agent and the browser harness it serves — see
[`agent/`](./agent/) and the architecture doc linked above. Iterating on either file
alone is an edit followed by `docker restart parley-recorder`, no image rebuild: the
mount is code, read fresh at container startup, not the kind of host/container data
exchange that needed the workarounds an earlier design iteration required.

Container ports are fixed: chromedriver on 9515 and the agent on 9516. The launcher
normally publishes the same host ports, but the host side and Chrome's container-local
agent URL are configured separately so another recorder can run alongside it:

```sh
PARLEY_GLYPH_CONTAINER=parley-recorder-speed \
PARLEY_GLYPH_WEBDRIVER_PORT=9525 \
PARLEY_GLYPH_AGENT_PORT=9526 \
  container/run.sh minimise --jobs 4
```

Each minimiser worker opens a separate Chrome instance in that one container. Its SKP
files live in a session-private subdirectory, so captures can safely overlap.
`fuzz_loop --jobs N` uses the same isolation and assigns each seed exactly once from a
shared counter. `minimise` resumes discovery-mode batches by default: a seed with both
`minimised.txt` and `minimised_mismatch.txt` is included in the report without opening
Chrome again. Pass `--force` to reprocess completed seeds; explicit case paths are
always processed.

## Threat model

**This container is not network-isolated.** `chromedriver --allowed-ips=` (empty —
Chrome for Testing 151's successor to `--whitelisted-ips`'s "allow all") is not itself
an access control; host-side `-p 127.0.0.1:...` publishing is. Only processes on the
host's own loopback interface can ever reach either port, regardless of what
chromedriver or the agent would accept from elsewhere. This is the same posture as
before (an earlier iteration reconstructed an equivalent check via
`CHROMEDRIVER_ALLOWED_IP` gateway autodetection, which is why that machinery is gone
rather than replaced) — accepted deliberately, since inputs here are self-generated
test HTML, not adversarial, and this pipeline never runs in CI, only locally/manually.

The Deno agent's permission flags (`--allow-net`, `--allow-read`, `--allow-write`,
`--allow-run`, all scoped to exactly what `agent.ts` needs — see `entrypoint.sh`) are
a second, narrower layer: even something that could reach the published port can only
do what those endpoints expose, which is deliberately "transport, not logic" (see the
architecture doc's endpoint table).

## Manual verification (do this once per image rebuild)

```sh
container/run.sh regenerate_goldens
```

should complete with `git diff --exit-code` empty on
`parley_tests/tests/glyph_positioning/` (a byte-identical no-op proves the image
still reproduces the checked-in corpus), followed by a short burn-in:

```sh
container/run.sh fuzz_loop --max-cases 100
```

with zero harness failures (mismatches are genuine parity findings, not harness bugs —
see the fuzz loop's own doc comment).
