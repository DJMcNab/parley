# Recorder agent: an in-container HTTP transport

Supersedes the `docker exec` + bind-mount transport described in
[`glyph-positioning-chrome-parity-phase4.md`](./glyph-positioning-chrome-parity-phase4.md)
and its "Environment" section. That document's *findings* (B1–B13, the strut fix, the
16.16/ceiling work) all remain true and are not repeated here — this is only the
transport the driver now speaks.

## What replaced what and why

| before | after |
| --- | --- |
| `docker exec` for `skp_parser` and cleanup | one HTTP service inside the container |
| two bind mounts (`/skp` out, `/harness` in), described by 4 host/container path pairs | one read-only code mount (`container/agent/` → `/agent`) |
| 6 env vars, 2 of them required | 2 env vars, 0 required |
| gvproxy gateway autodetection (`CHROMEDRIVER_ALLOWED_IP`) | `--allowed-ips=` (empty — allow any remote IP); host-side `-p 127.0.0.1:...` publishing is the actual access control |
| harness page loaded via `file://`, `--allow-file-access-from-files` | harness page served by the agent over `http://`, no such flag needed |
| B5's driver-side workaround: delete SKPs via `docker exec`, never from the host | no host-visible SKP boundary at all — the agent is the only thing that ever touches `/skp` |

The B5 finding — that a host-side `unlink` on the bind mount left the container's view
of the directory stale — is now moot rather than newly fixed: there is no bind mount
for `/skp` to go stale on. The agent deletes and lists it from inside the same
container process space it was always going to run in.

## Why the agent directory's mount is exempt from B5's pathology

`/agent` is mounted read-only and holds *code*, read once at container startup
(`agent.ts`, `harness.ts`, `harness.html`, `shared.ts`, the two `deno.json`s). B5's
staleness was specifically about a *data* exchange — the host deleting a file the
container was about to read fresh on every capture. Nothing here is written by one
side and read moments later by the other inside a single case's lifetime: updating the
agent or harness is an edit-and-`docker restart`, not a per-capture round trip.

## Container processes and ports

`entrypoint.sh` is a two-process supervisor (plain `bash`, which `debian:trixie-slim`
has): typecheck and bundle the harness, then start chromedriver and the Deno agent in
parallel, `wait -n` for either to exit, kill the other, exit. There is no WebDriver
proxy — chromedriver keeps its own published port, unchanged from before.

| process | port | published as |
| --- | --- | --- |
| chromedriver | 9515 | `127.0.0.1:9515` |
| agent | 9516 | `127.0.0.1:9516` |

Host and container port numbers are identical on purpose: Chrome *inside* the
container loads the harness page from the agent via container-localhost
(`http://127.0.0.1:9516/harness/harness.html`), and the driver on the *host* reaches
the same agent through the published port at the same URL. Keeping the numbers equal
means one URL string works on both sides of that boundary — only
`container/run.sh` and `Config` need to agree on it, not a fourth thing.

## The agent's HTTP API

All parsing and every intelligent decision (sfnt scanning, PostScript-name resolution,
the golden schema, the comparison predicate) stays in Rust. The agent
(`container/agent/agent.ts`) is deliberately dumb: it moves bytes and does the one
"is this a `layer_*.skp` name" check needed to keep it from touching arbitrary paths.

| method | path | does |
| --- | --- | --- |
| `PUT` | `/harness/<name>` | stores the body in memory (fonts, uploaded once at session start) |
| `GET` | `/harness/<name>` | serves, in priority order: the startup bundle for `harness.js`; `harness.html` from the mounted directory; otherwise the uploaded store |
| `DELETE` | `/skp` | removes `layer_*.skp` from the container-internal SKP directory |
| `GET` | `/skp` | JSON array of current `layer_*.skp` names, sorted |
| `GET` | `/skp/<name>` | raw SKP bytes |
| `GET` | `/skp/<name>/commands` | stdout of `skp_parser <path>` (the JSON command dump); a non-zero exit is a 5xx with stderr as the body |
| `GET` | `/skp/<name>/typeface?key=<data-key>` | stdout of `skp_parser <path> <key>` (the serialized typeface bytes) |

`<name>` must be a plain basename for `/harness/`, and must match `layer_*.skp` for
`/skp/` — the one path-traversal check this needs, not a security-theater exercise.
`<data-key>` (e.g. `data/0`) contains a slash, hence the query parameter — it is
percent-encoded by the Rust client and passed through verbatim by the agent, never
parsed on either side.

The SKP directory itself is unchanged in kind from before: a plain container-internal
directory created in the Dockerfile, owned by the `parley` user — no mount, no tmpfs.
The host simply never sees it directly any more; only the agent's `/skp` surface does.

## The launcher is the lockstep contract

There is deliberately **no version handshake and no protocol constant** between the
Rust driver and the browser harness. `container/run.sh` is what keeps them in lockstep
in practice: it builds the image if needed, restarts the container, waits for both
ports, then execs the requested recorder binary — so the code that talks to a
container is always the code that just (re)started it.

```sh
container/run.sh fuzz_loop --max-cases 300
container/run.sh regenerate_goldens
```

Iterating on the agent or harness alone (no Rust or image changes) is just an edit
followed by `docker restart parley-recorder` — the mount is read live.

## Rust side

- `src/agent.rs` — `AgentClient`, a blocking `ureq` client for the table above, called
  synchronously from the driver's async methods exactly as the old `docker exec` calls
  were (a handful of localhost round-trips per capture needs no async plumbing).
- `src/driver.rs` — `Config` is two URLs (`webdriver`, `agent`), both env-overridable
  (`PARLEY_GLYPH_WEBDRIVER`, `PARLEY_GLYPH_AGENT`), neither required. `Recorder`
  uploads fonts through the agent at `attach`, navigates to the agent's own
  `/harness/harness.html`, and drives capture/cleanup/fetch through it.
- `src/skp_json.rs` — pure parsing now: no `docker exec`, no `Command`. Fed by bytes
  the agent already fetched.
- `src/harness.rs` — still owns the `Case`/`Run` → CSS mapping and the payload
  builder; `renderAndCapture`'s payload lost its SKP-directory parameter, since
  `harness.ts` gets that from `shared.ts` directly (see below).

## Browser side

`container/agent/` holds both the agent and the harness, colocated because they share
one constant (`shared.ts`'s `SKP_DIR`) and are bundled/served by the same process:

- `agent.ts` — the HTTP service.
- `harness.ts` — the browser half, converted from the old `harness.js` to real
  TypeScript. Same envelope contract, same double `requestAnimationFrame` wait, same
  behavior — only the transport around it changed. `renderAndCapture` now imports
  `SKP_DIR` from `shared.ts` and calls `printToSkPicture` itself, rather than taking a
  directory argument the Rust driver would otherwise have to know.
- `harness.html` — unchanged in substance; still names `Roboto.ttf` via a relative
  `url()`, which now resolves against the agent's own `/harness/` route instead of a
  `file://` mount.
- `shared.ts` — the one constant (`SKP_DIR`) both files need.
- `deno.json` / `deno.harness.json` — `agent.ts` (and `shared.ts`, which it imports)
  type-checks under a Deno-flavoured `lib` (no `dom`: a stray `document` reference
  should be a compile error in a server process); `harness.ts` type-checks separately
  under a DOM-flavoured one (no `deno.ns`: none of that exists once bundled for the
  browser). `deno lint`/`deno fmt` cover the whole directory from the one default
  config, since neither is lib-sensitive.

Startup (in `entrypoint.sh`, before either long-lived process starts):

1. `deno check --config deno.harness.json harness.ts` — a type error fails container
   startup, never a mid-session request.
2. `deno bundle --platform=browser --format=iife harness.ts` → a single classic script,
   held at a container-internal temp path and served by the agent as
   `/harness/harness.js`.
3. `deno run --check agent.ts` (plain `--check`, not `--check=all`: `agent.ts` has no
   remote/npm imports for `=all` to additionally cover, and `=all` also type-checks
   Deno's own lib `.d.ts` files against whatever `lib` list is configured — which fails
   under a `dom`-less list, since they cross-reference `Window`).

Deno itself is a pinned static binary baked into the image (`ARG DENO_VERSION` in
`container/Dockerfile`), same download pattern as the Chrome for Testing artifacts:
plain HTTPS, `curl -f`, `unzip` failing loudly on a truncated download, no self-pinned
hash. `deno bundle`'s esbuild dependency is pre-warmed into the image at build time
(against a throwaway file) so the running container never needs a network round-trip
for it at startup.

## CI

One job (`.github/workflows/ci.yml`) runs `deno check`/`deno lint`/`deno fmt --check`
against `container/agent/`, with `denoland/setup-deno` pinned to the version read out
of the Dockerfile's `ARG DENO_VERSION` — a `grep`/`sed` step, not a second pin. No
Docker in CI, same as before: the recorder crate's own tests still need a live
container and are not part of the matrix.
