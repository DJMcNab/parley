#!/bin/bash
# Cold-start-to-running-binary launcher: builds the image if needed, starts (or
# restarts) the container, waits for both ports, then execs the requested recorder
# binary. `docker` is hardcoded (the user's docker-compat CLI is Podman; nothing here
# may branch on the engine). Usage:
#
#   container/run.sh fuzz_loop --max-cases 300
#   container/run.sh regenerate_goldens [filter]
#   container/run.sh minimise [--jobs N] [--out DIR] [case.txt ...]
#
# PARLEY_GLYPH_CONTAINER and PARLEY_GLYPH_{WEBDRIVER,AGENT}_PORT can be overridden
# to run alongside another recorder container.
set -eu

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <bin> [args...]  (bin is one of fuzz_loop, regenerate_goldens, minimise)" >&2
  exit 1
fi
bin="$1"
shift

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../.." && pwd)"
image="${PARLEY_GLYPH_IMAGE:-parley-glyph-recorder:latest}"
container="${PARLEY_GLYPH_CONTAINER:-parley-recorder}"
webdriver_port="${PARLEY_GLYPH_WEBDRIVER_PORT:-9515}"
agent_port="${PARLEY_GLYPH_AGENT_PORT:-9516}"

if ! docker image inspect "${image}" >/dev/null 2>&1; then
  echo "building ${image}..." >&2
  docker build -t "${image}" "${script_dir}"
fi

docker rm -f "${container}" >/dev/null 2>&1 || true

# Container ports stay fixed; host ports are configurable so independent recorder
# containers can coexist. Chrome uses the container-local agent URL exported below.
docker run -d --name "${container}" \
  -v "${script_dir}/agent:/agent:ro" \
  -p "127.0.0.1:${webdriver_port}:9515" \
  -p "127.0.0.1:${agent_port}:9516" \
  "${image}" >/dev/null

echo "waiting for chromedriver and the agent..." >&2
for port in "${webdriver_port}" "${agent_port}"; do
  for _ in $(seq 1 60); do
    if curl -s -o /dev/null "http://127.0.0.1:${port}/"; then
      break
    fi
    sleep 1
  done
  if ! curl -s -o /dev/null "http://127.0.0.1:${port}/"; then
    echo "port ${port} never answered; check \`docker logs ${container}\`" >&2
    exit 1
  fi
done

cd "${repo_root}"
export PARLEY_GLYPH_WEBDRIVER="http://127.0.0.1:${webdriver_port}"
export PARLEY_GLYPH_AGENT="http://127.0.0.1:${agent_port}"
# Chrome runs inside the container, where the agent retains its fixed internal port.
export PARLEY_GLYPH_BROWSER_AGENT="http://127.0.0.1:9516"
exec cargo run -p parley_glyph_positioning_recorder --bin "${bin}" -- "$@"
