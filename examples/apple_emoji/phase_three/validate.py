#!/usr/bin/env python3
# Copyright 2026 the Parley Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Dependency-free ChromeDriver validation harness for Phase Three.

Copied/adapted from `examples/apple_emoji/phase_two/validate.py` (phase_two is not
edited or imported).

Serves the trunk `dist/` build over HTTP, plus a dedicated `/noto-ref/...` route that
streams the *real* `NotoColorEmoji-Regular.ttf` straight from the repo (never copied into
`dist/`, never part of the app's own load path — see phase_three_plan.md §6.1). Drives
Chrome for Testing via ChromeDriver's W3C WebDriver protocol, waits for the page to
report "done", scrapes the per-emoji metrics JSON (`#data-phase-three`) and the in-page
real-Noto reference measurement JSON (`#data-noto-ref`), checks parity, and takes a
screenshot.

Usage:
    cd examples/apple_emoji/phase_three
    cargo run -p noto_advance_gen -- src/noto_advances.rs   # regenerate the table
    trunk build
    python3 validate.py
"""

import http.server
import json
import subprocess
import sys
import threading
import time
import urllib.request
import base64
import socket
from pathlib import Path

CHROMEDRIVER = "/Users/djmcnab/chromedriver/mac_arm-150.0.7871.46/chromedriver-mac-arm64/chromedriver"
CHROME_BINARY = (
    "/Users/djmcnab/chrome/mac_arm-150.0.7871.46/chrome-mac-arm64/"
    "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
)

HERE = Path(__file__).resolve().parent
DIST_DIR = HERE / "dist"
ARTIFACTS_DIR = HERE / "_artifacts"

# examples/apple_emoji/phase_three -> repo root
REPO_ROOT = HERE.parent.parent.parent
NOTO_FONT_PATH = REPO_ROOT / "parley_dev" / "assets" / "fonts" / "noto_color_emoji" / "NotoColorEmoji-Regular.ttf"

CHROMEDRIVER_PORT = 9517
HTTP_PORT = 8179

# Tolerances (px), per the plan's acceptance criteria.
TABLE_VS_NOTOREF_TOL = 0.5
EMOJI_SUM_PARITY_TOL = 1.0
LINE_WIDTH_PARITY_TOL = 1.0

# Independently (re-)measures each HTML row's baseline, each emoji's pen-left, and each
# drawn `.cmp-box`'s top/left, WITHOUT reusing index.html's `drawComparisonBoxes()`
# helper variables — so this check would have caught the "box positioned relative to
# the wrong containing block" bug (box_top computed relative to the text row, but
# placed relative to the outer `.row` wrapper, which also contains the label above).
BOX_GEOMETRY_TOL = 1.0
BOX_ASCENT_JS = 950 / 1024 * 48
BOX_GEOMETRY_SCRIPT = """
const EMOJI = ["\\u{1F600}", "\\u{1F389}"];
const out = [];
for (const rowId of ['native', 'ref-row']) {
  const row = document.getElementById(rowId);
  if (!row) continue;
  const container = row.closest('.row') || row;
  const containerRect = container.getBoundingClientRect();

  const probe = document.createElement('span');
  probe.style.display = 'inline-block';
  probe.style.width = '0';
  probe.style.height = '0';
  probe.style.verticalAlign = 'baseline';
  row.appendChild(probe);
  const baselineY = probe.getBoundingClientRect().bottom - containerRect.top;
  probe.remove();

  const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
  let node;
  const emojiLefts = [];
  while ((node = walker.nextNode())) {
    const text = node.nodeValue;
    for (const ch of EMOJI) {
      let idx = text.indexOf(ch);
      while (idx !== -1) {
        const range = document.createRange();
        range.setStart(node, idx);
        range.setEnd(node, idx + ch.length);
        const rect = range.getBoundingClientRect();
        emojiLefts.push(rect.left - containerRect.left);
        idx = text.indexOf(ch, idx + ch.length);
      }
    }
  }

  const boxes = Array.from(container.querySelectorAll('.cmp-box')).map((box) => {
    const r = box.getBoundingClientRect();
    return { top: r.top - containerRect.top, left: r.left - containerRect.left };
  });

  out.push({ rowId, baselineY, emojiLefts, boxes });
}
return JSON.stringify(out);
"""


def find_free_port_or(default):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind(("127.0.0.1", default))
            return default
        except OSError:
            s.bind(("127.0.0.1", 0))
            return s.getsockname()[1]


class Handler(http.server.SimpleHTTPRequestHandler):
    """Serves `dist/` as the document root, plus a `/noto-ref/...` route that streams
    the real Noto Color Emoji font straight from the repo path (validation only —
    never baked into `dist/`, see module docstring)."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(DIST_DIR), **kwargs)

    def log_message(self, fmt, *args):
        pass

    def do_GET(self):
        if self.path.startswith("/noto-ref/"):
            self._serve_noto_ref()
            return
        super().do_GET()

    def _serve_noto_ref(self):
        if not NOTO_FONT_PATH.exists():
            self.send_error(404, "Noto font not found on disk")
            return
        data = NOTO_FONT_PATH.read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "font/ttf")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def serve_dist(port):
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    return httpd


def wd_request(base_url, method, path, body=None):
    url = f"{base_url}{path}"
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode("utf-8"))


def start_chromedriver():
    proc = subprocess.Popen(
        [CHROMEDRIVER, f"--port={CHROMEDRIVER_PORT}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    base = f"http://127.0.0.1:{CHROMEDRIVER_PORT}"
    for _ in range(50):
        try:
            urllib.request.urlopen(f"{base}/status", timeout=1)
            return proc, base
        except Exception:
            time.sleep(0.1)
    raise RuntimeError("chromedriver did not start in time")


def new_session(base, headless):
    args = ["--window-size=900,1100", "--force-color-profile=srgb"]
    if headless:
        args.insert(0, "--headless=new")
    payload = {
        "capabilities": {
            "alwaysMatch": {
                "goog:chromeOptions": {
                    "binary": CHROME_BINARY,
                    "args": args,
                }
            }
        }
    }
    result = wd_request(base, "POST", "/session", payload)
    return result["value"]["sessionId"]


def run_session(base, session_id, http_port):
    wd_request(base, "POST", f"/session/{session_id}/url", {"url": f"http://127.0.0.1:{http_port}"})

    deadline = time.time() + 30
    status_text = ""
    while time.time() < deadline:
        result = wd_request(
            base,
            "POST",
            f"/session/{session_id}/execute/sync",
            {"script": "return document.getElementById('status').textContent;", "args": []},
        )
        status_text = result.get("value") or ""
        if status_text.startswith("done") or status_text.startswith("FAILED"):
            break
        time.sleep(0.25)

    metrics_result = wd_request(
        base,
        "POST",
        f"/session/{session_id}/execute/sync",
        {
            "script": "return document.getElementById('data-phase-three').textContent;",
            "args": [],
        },
    )
    metrics_text = metrics_result.get("value") or ""
    metrics = None
    if metrics_text:
        try:
            metrics = json.loads(metrics_text)
        except json.JSONDecodeError:
            metrics = None

    # Wait a little longer for the (async, independent) NotoRef in-page reference
    # measurement script in index.html to finish (it loads the real 25 MB font).
    noto_ref = None
    deadline = time.time() + 30
    while time.time() < deadline:
        noto_ref_result = wd_request(
            base,
            "POST",
            f"/session/{session_id}/execute/sync",
            {
                "script": "return document.getElementById('data-noto-ref').textContent;",
                "args": [],
            },
        )
        noto_ref_text = noto_ref_result.get("value") or ""
        if noto_ref_text:
            try:
                noto_ref = json.loads(noto_ref_text)
                break
            except json.JSONDecodeError:
                pass
        time.sleep(0.25)

    box_geometry_result = wd_request(
        base,
        "POST",
        f"/session/{session_id}/execute/sync",
        {"script": BOX_GEOMETRY_SCRIPT, "args": []},
    )
    box_geometry = box_geometry_result.get("value") or None

    screenshot_result = wd_request(base, "GET", f"/session/{session_id}/screenshot")
    png_bytes = base64.b64decode(screenshot_result["value"])

    return status_text, metrics, noto_ref, box_geometry, png_bytes


def close_session(base, session_id):
    try:
        wd_request(base, "DELETE", f"/session/{session_id}")
    except Exception:
        pass


def attempt(base, http_port, headless):
    session_id = new_session(base, headless)
    try:
        return run_session(base, session_id, http_port)
    finally:
        close_session(base, session_id)


def check_box_geometry(box_geometry_text):
    """Independently validates each row's `.cmp-box` position against its own baseline
    and emoji pen-left. Returns (passed, report_lines)."""
    lines = []
    if not box_geometry_text:
        return False, ["  (no box geometry captured)"]
    try:
        rows = json.loads(box_geometry_text)
    except json.JSONDecodeError:
        return False, ["  (box geometry JSON did not parse)"]

    passed = True
    if not rows:
        return False, ["  (no rows measured)"]

    for row in rows:
        row_id = row["rowId"]
        baseline_y = row["baselineY"]
        emoji_lefts = row["emojiLefts"]
        boxes = row["boxes"]
        if len(boxes) != len(emoji_lefts):
            lines.append(
                f"  {row_id}: WARNING box count ({len(boxes)}) != emoji count ({len(emoji_lefts)})"
            )
            passed = False
            continue
        for box, emoji_left in zip(boxes, emoji_lefts):
            top_offset = box["top"] - baseline_y
            top_diff = abs(top_offset - (-BOX_ASCENT_JS))
            left_diff = abs(box["left"] - emoji_left)
            lines.append(
                f"  {row_id}: box_top-baseline={top_offset:.3f} (want {-BOX_ASCENT_JS:.3f}, "
                f"diff={top_diff:.3f}); box_left={box['left']:.3f} emoji_left={emoji_left:.3f} "
                f"(diff={left_diff:.3f})"
            )
            if top_diff > BOX_GEOMETRY_TOL or left_diff > BOX_GEOMETRY_TOL:
                passed = False
    return passed, lines


def main():
    if not DIST_DIR.exists():
        print(f"error: {DIST_DIR} does not exist; run `trunk build` first.", file=sys.stderr)
        return 1
    if not NOTO_FONT_PATH.exists():
        print(f"error: {NOTO_FONT_PATH} does not exist.", file=sys.stderr)
        return 1

    ARTIFACTS_DIR.mkdir(exist_ok=True)

    http_port = find_free_port_or(HTTP_PORT)
    httpd = serve_dist(http_port)

    chromedriver_proc, cd_base = start_chromedriver()
    try:
        status_text, metrics, noto_ref, box_geometry, png_bytes = attempt(cd_base, http_port, headless=True)
        mode = "headless"
        if not (metrics and metrics.get("emoji") and noto_ref):
            print("headless capture failed or inconclusive; retrying headed…")
            status_text, metrics, noto_ref, box_geometry, png_bytes = attempt(cd_base, http_port, headless=False)
            mode = "headed"
    finally:
        chromedriver_proc.terminate()
        try:
            chromedriver_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            chromedriver_proc.kill()
        httpd.shutdown()

    screenshot_path = ARTIFACTS_DIR / "phase_three.png"
    screenshot_path.write_bytes(png_bytes)

    print(f"mode: {mode}")
    print(f"status: {status_text}")
    print(f"screenshot: {screenshot_path}")
    print("metrics (from the Rust/wasm app, #data-phase-three):")
    print(json.dumps(metrics, indent=2) if metrics else "  (none captured)")
    print("noto-ref (in-page real-Noto reference measurement, #data-noto-ref):")
    print(json.dumps(noto_ref, indent=2) if noto_ref else "  (none captured)")

    passed = bool(status_text.startswith("done") and metrics and metrics.get("emoji") and noto_ref)

    if noto_ref is not None:
        # Informational: confirm the reference row actually rendered with real Noto
        # Color Emoji (not a system-font fallback). Does not affect pass/fail — the
        # primary/secondary parity checks below already depend on NotoRef having
        # loaded, so a `False` here would show up as a parity failure too.
        noto_loaded = noto_ref.get("noto_loaded")
        print(f"reference row NotoRef loaded: {noto_loaded}")
        if not noto_loaded:
            print("NOTE: reference row fell back to a system emoji font (harness route not reached?)")

    if passed:
        # 1. Per-emoji table check: embedded-table advance vs real-NotoRef measureText.
        for e in metrics["emoji"]:
            ref_w = noto_ref["per_emoji"].get(e["char"])
            if ref_w is None:
                print(f"WARNING: no NotoRef reference measurement for {e['char']!r}")
                passed = False
                continue
            diff = abs(ref_w - e["noto_advance_px"])
            print(
                f"  {e['char']!r}: table={e['noto_advance_px']:.3f} NotoRef={ref_w:.3f} "
                f"apple={e['apple_advance_width']:.3f} h_offset={e.get('h_offset', float('nan')):.4f} "
                f"capture_font_size={e.get('capture_font_size', float('nan')):.3f} diff={diff:.3f}"
            )
            if diff > TABLE_VS_NOTOREF_TOL:
                print(f"WARNING: table-vs-NotoRef advance mismatch for {e['char']!r}: diff={diff:.3f}")
                passed = False

        # 2. Emoji-advance-sum parity (PRIMARY acceptance metric).
        sum_diff = abs(metrics["emoji_advance_sum"] - noto_ref["emoji_advance_sum"])
        print(
            f"  emoji_advance_sum: parley={metrics['emoji_advance_sum']:.3f} "
            f"noto_ref={noto_ref['emoji_advance_sum']:.3f} diff={sum_diff:.3f}"
        )
        if sum_diff > EMOJI_SUM_PARITY_TOL:
            print(f"WARNING: emoji-advance-sum parity failed: diff={sum_diff:.3f}")
            passed = False

        # 3. Full-line-width parity (secondary acceptance metric).
        line_diff = abs(metrics["layout_width"] - noto_ref["line_width"])
        print(
            f"  line_width: parley={metrics['layout_width']:.3f} "
            f"noto_ref={noto_ref['line_width']:.3f} diff={line_diff:.3f}"
        )
        if line_diff > LINE_WIDTH_PARITY_TOL:
            print(f"NOTE: line-width parity (secondary) exceeded tolerance: diff={line_diff:.3f}")

        # 4. Comparison-box geometry (HTML overlay rows 1/3): box must tightly bracket
        # each row's own emoji, anchored to that row's own baseline/pen-left.
        box_geom_passed, box_geom_lines = check_box_geometry(box_geometry)
        print("comparison-box geometry (rows 1/3):")
        for line in box_geom_lines:
            print(line)
        if not box_geom_passed:
            print("WARNING: comparison-box geometry check failed")
            passed = False

    print()
    print("RESULT:", "PASS" if passed else "FAIL")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
