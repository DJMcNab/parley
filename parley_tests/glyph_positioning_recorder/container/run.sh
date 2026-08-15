#!/bin/bash
# Cold-start-to-running-binary launcher: builds the image if needed, starts (or
# restarts) the container, waits for both ports, then execs the requested recorder
# binary. `docker` is hardcoded (the user's docker-compat CLI is Podman; nothing here
# may branch on the engine). Usage:
#
#   container/run.sh fuzz_loop --max-cases 300
#   container/run.sh regenerate_goldens [filter]
#   container/run.sh minimise [--out DIR] [case.txt ...]
set -eu

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <bin> [args...]  (bin is one of fuzz_loop, regenerate_goldens, minimise)" >&2
  exit 1
fi
bin="$1"
shift

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../../.." && pwd)"
image="parley-glyph-recorder:latest"
container="parley-recorder"

if ! docker image inspect "${image}" >/dev/null 2>&1; then
  echo "building ${image}..." >&2
  docker build -t "${image}" "${script_dir}"
fi

docker rm -f "${container}" >/dev/null 2>&1 || true

# Both publishes use identical host/container port numbers: Chrome inside the
# container loads the harness page from the agent via container-localhost, so keeping
# the numbers equal means the same URL (`http://127.0.0.1:<port>/...`) works on both
# sides of the boundary, and only the driver's config needs to know it.
docker run -d --name "${container}" \
  -v "${script_dir}/agent:/agent:ro" \
  -p 127.0.0.1:9515:9515 \
  -p 127.0.0.1:9516:9516 \
  "${image}" >/dev/null

echo "waiting for chromedriver and the agent..." >&2
for port in 9515 9516; do
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
exec cargo run -p parley_glyph_positioning_recorder --bin "${bin}" -- "$@"
