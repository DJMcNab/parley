#!/bin/bash
# A two-process supervisor: chromedriver and the Deno agent, run in parallel;
# whichever exits first takes the other down with it.
#
# `--allowed-ips=` (empty value) is chromedriver's "allow any remote IP" successor to
# the old `--whitelisted-ips`. This container is not network-isolated (see
# ../README.md's threat model), so host-side `-p 127.0.0.1:...` publishing is the real
# access control: only processes on the host's loopback interface can ever reach this
# port at all, regardless of what chromedriver itself would accept. That is what makes
# an empty `--allowed-ips` acceptable rather than a downgrade — the previous
# `CHROMEDRIVER_ALLOWED_IP` gateway-autodetection dance existed only to reconstruct a
# check this makes unnecessary.
set -eu

AGENT_DIR=/agent
HARNESS_BUNDLE=/tmp/harness-bundle.js

# Steps (b) and (c) run before (a) starts the agent: the bundle has to exist before
# the agent can read it at startup.

# (b) Typecheck the browser half under its DOM-flavoured config. A type error here
# must fail container startup loudly, never surface as a mid-session request failure.
deno check --config "${AGENT_DIR}/deno.harness.json" "${AGENT_DIR}/harness.ts"

# (c) Bundle it to a single classic script and hand the result to the agent, which
# serves it as `/harness/harness.js`. `--platform=browser` targets browser globals,
# `--format=iife` keeps it loadable via a plain `<script src>` (harness.html does not
# use `type="module"`). No `--check` here: the `deno check` above already typechecked
# this exact file under the right (DOM) config, and `deno bundle` does not type-check
# by default, so there is no risk of it silently re-checking under the wrong one.
deno bundle --platform=browser --format=iife \
  --output "${HARNESS_BUNDLE}" "${AGENT_DIR}/harness.ts"

/opt/chromedriver-linux64/chromedriver \
  --port=9515 \
  --allowed-ips= \
  --allowed-origins=* &
chromedriver_pid=$!

# (a) `--check` so a type error in `agent.ts` (or `shared.ts`, which it imports) fails
# container startup rather than the first request that exercises it. Plain `--check`,
# not `--check=all`: the two are equivalent here since `agent.ts` has no remote/npm
# imports for `=all` to additionally cover, and `=all` also type-checks Deno's own
# lib `.d.ts` files against our lib list, which fails — they cross-reference `Window`,
# which only resolves once `dom` is in scope, defeating the point of leaving it out.
PARLEY_HARNESS_BUNDLE_PATH="${HARNESS_BUNDLE}" \
  deno run --check \
    --allow-net=0.0.0.0:9516 \
    --allow-env=PARLEY_HARNESS_BUNDLE_PATH \
    --allow-read="${AGENT_DIR},/skp,${HARNESS_BUNDLE}" \
    --allow-write=/skp \
    --allow-run=/usr/local/bin/skp_parser \
    "${AGENT_DIR}/agent.ts" &
agent_pid=$!

# Whichever process exits first, the other is no longer useful: a dead chromedriver
# means no more captures, and a dead agent means no more SKP access or page serving.
wait -n "${chromedriver_pid}" "${agent_pid}"
kill "${chromedriver_pid}" "${agent_pid}" 2>/dev/null || true
wait
