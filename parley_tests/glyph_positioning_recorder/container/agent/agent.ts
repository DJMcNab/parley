// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// The in-container HTTP agent: the only non-WebDriver transport between the Rust
// driver and this container. It replaces the old `docker exec` + bind-mount surface
// (font/page staging, SKP listing, SKP cleanup, `skp_parser` invocation) with a small
// HTTP API on port 9516.
//
// Design for dumbness: this file is transport, not logic. All parsing and every
// intelligent decision (sfnt scanning, PostScript-name resolution, the golden schema,
// …) stays in Rust, fed by the bytes this serves. See "The agent" and "Agent HTTP API"
// in `doc/glyph-positioning-recorder-agent.md`.
//
// Endpoints:
//   PUT    /harness/<name>            store the body in memory (fonts, at session start)
//   GET    /harness/<name>            harness.js bundle | harness.html | an upload
//   PUT    /capture/<id>              create a session-private SKP directory
//   DELETE /capture/<id>              remove its layer_*.skp files
//   GET    /capture/<id>              JSON array of its layer_*.skp names, sorted
//   GET    /capture/<id>/<name>       raw SKP bytes
//   GET    /capture/<id>/<name>/commands
//   GET    /capture/<id>/<name>/typeface?key=<data-key>
//
// Run with `deno run --check`, per the entrypoint, so a type error here fails
// container startup rather than a mid-session request.

import { SKP_DIR } from "./shared.ts";

/** The port this agent listens on. See `container/README.md` for the paired publish. */
const PORT = 9516;

/** The read-only bind mount this file and `harness.html` live in. */
const AGENT_DIR = "/agent";

/** The `skp_parser` binary baked into the image (see `../Dockerfile`). */
const SKP_PARSER = "/usr/local/bin/skp_parser";

/**
 * Where the entrypoint's `deno bundle` step wrote the browser bundle of `harness.ts`,
 * read once at startup and served as `/harness/harness.js` for the rest of the
 * container's life. A missing file here is a startup failure, not a per-request one.
 */
const HARNESS_BUNDLE_PATH = Deno.env.get("PARLEY_HARNESS_BUNDLE_PATH") ??
  "/tmp/harness-bundle.js";

/**
 * Uploaded harness resources, keyed by upload name.
 *
 * Only fonts transit this map in practice — the driver uploads them once per session
 * (see `harness.rs`'s `font_file_name`) — but the map itself has no opinion about what
 * gets stored in it.
 */
const uploads = new Map<string, Uint8Array>();

const harnessBundle: Uint8Array = await Deno.readFile(HARNESS_BUNDLE_PATH);

/** A bare file name: no path separators, no `.`/`..`. */
function isPlainBasename(name: string): boolean {
  return /^[A-Za-z0-9._-]+$/.test(name) && name !== "." && name !== "..";
}

/** The shape of one of `printToSkPicture`'s outputs, mirroring the Rust driver's check. */
function isLayerSkpName(name: string): boolean {
  return (
    isPlainBasename(name) && name.startsWith("layer_") && name.endsWith(".skp")
  );
}

/** A driver-generated, path-safe browser-session identifier. */
function isCaptureId(id: string): boolean {
  return /^capture-[A-Za-z0-9_-]+$/.test(id);
}

function captureDir(id: string): string {
  return `${SKP_DIR}/${id}`;
}

function contentTypeFor(name: string): string {
  if (name.endsWith(".html")) return "text/html";
  if (name.endsWith(".js")) return "text/javascript";
  if (name.endsWith(".ttf")) return "font/ttf";
  return "application/octet-stream";
}

/** Runs `skp_parser` with `args`, returning its stdout or throwing its stderr. */
async function runSkpParser(
  args: string[],
): Promise<Uint8Array> {
  const command = new Deno.Command(SKP_PARSER, {
    args,
    stdout: "piped",
    stderr: "piped",
  });
  const { code, stdout, stderr } = await command.output();
  if (code !== 0) {
    throw new Error(
      new TextDecoder().decode(stderr).trim() ||
        `skp_parser exited with status ${code}`,
    );
  }
  return stdout;
}

function textResponse(
  body: string,
  status = 200,
  contentType = "text/plain",
): Response {
  return new Response(body, {
    status,
    headers: { "content-type": contentType },
  });
}

/** `.slice()` gives a response body its own `ArrayBuffer`, satisfying `BodyInit`. */
function bytesResponse(
  bytes: Uint8Array,
  status = 200,
  contentType = "application/octet-stream",
): Response {
  return new Response(bytes.slice(), {
    status,
    headers: { "content-type": contentType },
  });
}

/**
 * Serves, in priority order: the startup bundle for `harness.js`, `harness.html` from
 * the mounted directory, otherwise the uploaded store (fonts).
 */
async function handleHarnessGet(name: string): Promise<Response> {
  if (!isPlainBasename(name)) {
    return textResponse("invalid harness file name", 400);
  }
  if (name === "harness.js") {
    return bytesResponse(harnessBundle, 200, "text/javascript");
  }
  if (name === "harness.html") {
    try {
      const bytes = await Deno.readFile(`${AGENT_DIR}/harness.html`);
      return bytesResponse(bytes, 200, "text/html");
    } catch {
      return textResponse(
        "harness.html not found in the mounted agent directory",
        404,
      );
    }
  }
  const uploaded = uploads.get(name);
  if (uploaded === undefined) {
    return textResponse(`no uploaded harness resource named ${name}`, 404);
  }
  return bytesResponse(uploaded, 200, contentTypeFor(name));
}

/** Stores the request body under `name`, for later `GET /harness/<name>`. */
async function handleHarnessPut(
  name: string,
  request: Request,
): Promise<Response> {
  if (!isPlainBasename(name)) {
    return textResponse("invalid harness file name", 400);
  }
  const bytes = new Uint8Array(await request.arrayBuffer());
  uploads.set(name, bytes);
  return textResponse("ok", 200);
}

async function handleCaptureInit(id: string): Promise<Response> {
  await Deno.mkdir(captureDir(id), { recursive: true });
  return textResponse("ok", 200);
}

async function handleSkpList(id: string): Promise<Response> {
  const names: string[] = [];
  for await (const entry of Deno.readDir(captureDir(id))) {
    if (entry.isFile && isLayerSkpName(entry.name)) {
      names.push(entry.name);
    }
  }
  names.sort();
  return new Response(JSON.stringify(names), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

/** Removes this session's whole capture directory; printToSkPicture recreates it. */
async function handleSkpClear(id: string): Promise<Response> {
  try {
    await Deno.remove(captureDir(id), { recursive: true });
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) {
      throw error;
    }
  }
  return textResponse("ok", 200);
}

async function handleSkpGet(id: string, name: string): Promise<Response> {
  if (!isLayerSkpName(name)) {
    return textResponse("invalid skp file name", 400);
  }
  try {
    const bytes = await Deno.readFile(`${captureDir(id)}/${name}`);
    return bytesResponse(bytes, 200, "application/octet-stream");
  } catch {
    return textResponse(`no such skp: ${name}`, 404);
  }
}

async function handleSkpCommands(id: string, name: string): Promise<Response> {
  if (!isLayerSkpName(name)) {
    return textResponse("invalid skp file name", 400);
  }
  try {
    const stdout = await runSkpParser([`${captureDir(id)}/${name}`]);
    return bytesResponse(stdout, 200, "application/json");
  } catch (error) {
    return textResponse(String((error as Error)?.message ?? error), 500);
  }
}

/** `key` (e.g. `data/0`) is passed to `skp_parser` verbatim — never parsed here. */
async function handleSkpTypeface(
  id: string,
  name: string,
  key: string | null,
): Promise<Response> {
  if (!isLayerSkpName(name)) {
    return textResponse("invalid skp file name", 400);
  }
  if (key === null || key.length === 0) {
    return textResponse("missing `key` query parameter", 400);
  }
  try {
    const stdout = await runSkpParser([`${captureDir(id)}/${name}`, key]);
    return bytesResponse(stdout, 200, "application/octet-stream");
  } catch (error) {
    return textResponse(String((error as Error)?.message ?? error), 500);
  }
}

function handler(request: Request): Promise<Response> {
  const url = new URL(request.url);
  const segments = url.pathname.split("/").filter((segment) =>
    segment.length > 0
  );

  if (segments[0] === "harness" && segments.length === 2) {
    const name = segments[1];
    if (request.method === "GET") return handleHarnessGet(name);
    if (request.method === "PUT") return handleHarnessPut(name, request);
  }
  if (segments[0] === "capture" && segments.length >= 2) {
    const id = segments[1];
    if (!isCaptureId(id)) {
      return Promise.resolve(textResponse("invalid capture id", 400));
    }
    if (segments.length === 2) {
      if (request.method === "PUT") return handleCaptureInit(id);
      if (request.method === "GET") return handleSkpList(id);
      if (request.method === "DELETE") return handleSkpClear(id);
    }
    if (segments.length === 3 && request.method === "GET") {
      return handleSkpGet(id, segments[2]);
    }
    if (
      segments.length === 4 && segments[3] === "commands" &&
      request.method === "GET"
    ) {
      return handleSkpCommands(id, segments[2]);
    }
    if (
      segments.length === 4 && segments[3] === "typeface" &&
      request.method === "GET"
    ) {
      return handleSkpTypeface(id, segments[2], url.searchParams.get("key"));
    }
  }
  return Promise.resolve(
    textResponse(`no route for ${request.method} ${url.pathname}`, 404),
  );
}

console.log(`glyph-positioning agent listening on :${PORT}`);
await Deno.serve({ port: PORT, hostname: "0.0.0.0" }, handler).finished;
