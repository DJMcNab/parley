#!/usr/bin/env python3
# Copyright 2026 the Parley Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Dependency-light ChromeDriver validation harness for Phase Four.

Copied/adapted from `examples/apple_emoji/phase_three/validate.py` (phase_three is not
edited or imported). Uses different ports to avoid colliding with a possibly-running
phase_three server.

Serves the trunk `dist/` build over HTTP, plus a dedicated `/noto-ref/...` route that
streams the *real* `NotoColorEmoji-Regular.ttf` straight from the repo (never copied into
`dist/`, never part of the app's own load path). Drives Chrome for Testing via
ChromeDriver's W3C WebDriver protocol, waits for the page to report "done", scrapes the
per-emoji metrics JSON (`#data-phase-four`) and the in-page real-Noto reference JSON
(`#data-noto-ref`), checks:
  1. Full line width vs real-Noto reference, <= 1px.
  2. Per-emoji advance vs real-Noto reference, <= 0.5px.
  3. Baseline placement (user ruling, option A: global downward em-relative shift,
     supersedes the earlier "baseline == line_baseline" ruling): the app blits each
     emoji's own alphabetic baseline onto the shared Parley baseline *plus* a fixed
     `APPLE_TO_NOTO_BASELINE_SHIFT_EM * font_size` downward shift, so the emoji sit in
     Noto's vertical band. The exported `baseline` (per-emoji, the emoji's actual
     blitted baseline) must equal exported `line_baseline + v_shift_px` exactly — both
     `line_baseline` and `v_shift_px` come straight from the app's own
     `#data-phase-four` JSON, so this is an exact check (the shift is intentional and
     verified, not drift), not a pixel-tolerance one. The alpha-threshold ink bbox from
     the screenshot is still computed and printed as *informational* output.
  4. Payload audit: `dist/` contains no Apple font, no full Noto (only the fake font).
Takes a screenshot to `_artifacts/phase_four.png`.

Usage:
    cd examples/apple_emoji/phase_four
    cargo run -p fake_font_gen   # regenerate fake_noto.ttf + fake_noto_meta.rs
    trunk build
    python3 validate.py
"""

import http.server
import io
import json
import subprocess
import sys
import threading
import time
import urllib.request
import base64
import socket
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    Image = None

CHROMEDRIVER = "/Users/djmcnab/chromedriver/mac_arm-150.0.7871.46/chromedriver-mac-arm64/chromedriver"
CHROME_BINARY = (
    "/Users/djmcnab/chrome/mac_arm-150.0.7871.46/chrome-mac-arm64/"
    "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
)

HERE = Path(__file__).resolve().parent
DIST_DIR = HERE / "dist"
ARTIFACTS_DIR = HERE / "_artifacts"

# examples/apple_emoji/phase_four -> repo root
REPO_ROOT = HERE.parent.parent.parent
NOTO_FONT_PATH = REPO_ROOT / "parley_dev" / "assets" / "fonts" / "noto_color_emoji" / "NotoColorEmoji-Regular.ttf"

# Different ports from phase_three (8179 / 9517) so both can run concurrently.
CHROMEDRIVER_PORT = 9527
HTTP_PORT = 8189

TABLE_VS_NOTOREF_TOL = 0.5
EMOJI_SUM_PARITY_TOL = 1.0
LINE_WIDTH_PARITY_TOL = 1.0
# Baseline placement is now an exact equality check on the app's own exported JSON (see
# module docstring, point 3) — this tiny epsilon only absorbs float round-tripping through
# JSON serialization, not a real visual tolerance.
BASELINE_EXACT_TOL_PX = 0.01
# Tolerance for the HTML-overlay `.cmp-box` (rows 1/3) position check: each box's rendered
# top must be `BOX_ASCENT` px above its row's own baseline, and its left must match the
# emoji's pen-left, both within ~1px (sub-pixel layout rounding).
CMP_BOX_TOL_PX = 1.0


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
    the real Noto Color Emoji font straight from the repo path (validation only — never
    bundled by trunk/dist, see module docstring)."""

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
            "script": "return document.getElementById('data-phase-four').textContent;",
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

    # Canvas bounding rect (device px, for mapping canvas-local x/baseline to the
    # full-page screenshot's pixel coordinates).
    rect_result = wd_request(
        base,
        "POST",
        f"/session/{session_id}/execute/sync",
        {
            "script": "const r = document.getElementById('parley').getBoundingClientRect();"
            "return {left: r.left, top: r.top, width: r.width, height: r.height, dpr: window.devicePixelRatio};",
            "args": [],
        },
    )
    canvas_rect = rect_result.get("value") or {"left": 0, "top": 0, "dpr": 1}

    # In-page measurement of the HTML-overlay `.cmp-box` divs (rows 1/3), to check they're
    # anchored to their own row's baseline/pen-left rather than positioned relative to some
    # other element's coordinate space (the historical bug: boxes drawn relative to the
    # outer `.row` container's positioned-ancestor origin but computed relative to the text
    # line's own rect, landing up in the label text above). For each row: re-measure the
    # baseline the same way `drawComparisonBoxes` does (a transient zero-size
    # `vertical-align: baseline` probe), then compare each existing `.cmp-box`'s rendered
    # `top`/`left` (via `getBoundingClientRect`, container-relative) against the expected
    # `baseline - BOX_ASCENT` / emoji pen-left.
    cmp_box_script = """
      const EMOJI = ["\\u{1F600}", "\\u{1F389}"];
      const BOX_ASCENT = 950 / 1024 * 48;
      const out = [];
      for (const rowId of ['native', 'ref-row']) {
        const row = document.getElementById(rowId);
        if (!row) continue;
        const container = row.closest('.row') || row;
        const containerRect = container.getBoundingClientRect();

        const probe = document.createElement('span');
        probe.className = 'baseline-probe';
        row.appendChild(probe);
        const baselineY = probe.getBoundingClientRect().bottom - containerRect.top;
        probe.remove();

        const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
        const penLefts = [];
        let node;
        while ((node = walker.nextNode())) {
          const text = node.nodeValue;
          for (const ch of EMOJI) {
            let idx = text.indexOf(ch);
            while (idx !== -1) {
              const range = document.createRange();
              range.setStart(node, idx);
              range.setEnd(node, idx + ch.length);
              const rect = range.getBoundingClientRect();
              penLefts.push(rect.left - containerRect.left);
              idx = text.indexOf(ch, idx + ch.length);
            }
          }
        }

        const boxes = Array.from(row.querySelectorAll('.cmp-box')).map((box) => {
          const r = box.getBoundingClientRect();
          return { top: r.top - containerRect.top, left: r.left - containerRect.left };
        });

        out.push({ rowId, baselineY, boxAscentTarget: baselineY - BOX_ASCENT, penLefts, boxes });
      }
      return out;
    """
    cmp_box_result = wd_request(
        base,
        "POST",
        f"/session/{session_id}/execute/sync",
        {"script": cmp_box_script, "args": []},
    )
    cmp_box_info = cmp_box_result.get("value") or []

    screenshot_result = wd_request(base, "GET", f"/session/{session_id}/screenshot")
    png_bytes = base64.b64decode(screenshot_result["value"])

    return status_text, metrics, noto_ref, png_bytes, canvas_rect, cmp_box_info


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


def _is_comparison_box_color(r, g, b):
    """True if `(r, g, b)` looks like the magenta comparison-box outline (`rgba(255, 0,
    255, 0.6)` alpha-blended over the opaque white background, i.e. approximately `(255,
    102, 255)`) rather than real emoji ink. The box is now drawn around every emoji (see
    `render.rs::draw_comparison_box` / `index.html`'s `.cmp-box` overlay), and its outline
    sits inside the same x/y region these informational ink-aspect helpers sample, so it
    must be excluded or it would be picked up as "ink" and skew the measured bbox."""
    return r > 230 and b > 230 and g < 180


def ink_top_bottom_in_column(img, x0, x1, y_search_top, y_search_bottom, alpha_thresh=245):
    """Alpha-threshold ink bbox (rows with a non-background pixel) within the column
    `[x0, x1)` of a screenshot region `[y_search_top, y_search_bottom)`, assuming an
    opaque white background (so "ink" = any pixel that isn't near-white), ignoring the
    magenta comparison-box outline (see `_is_comparison_box_color`)."""
    x0 = max(0, int(x0))
    x1 = min(img.width, int(x1))
    y0 = max(0, int(y_search_top))
    y1 = min(img.height, int(y_search_bottom))
    if x1 <= x0 or y1 <= y0:
        return None
    region = img.crop((x0, y0, x1, y1)).convert("RGB")
    px = region.load()
    top, bottom = None, None
    for y in range(region.height):
        row_has_ink = False
        for x in range(region.width):
            r, g, b = px[x, y]
            if (r < alpha_thresh or g < alpha_thresh or b < alpha_thresh) and not _is_comparison_box_color(r, g, b):
                row_has_ink = True
                break
        if row_has_ink:
            if top is None:
                top = y
            bottom = y
    if top is None:
        return None
    return (top + y0, bottom + y0)


def ink_bbox_in_region(img, x0, x1, y0, y1, alpha_thresh=245):
    """Alpha-threshold ink bounding box `(left, top, right, bottom)` (inclusive) within
    the screenshot region `[x0, x1) x [y0, y1)`, or `None` if no ink found. Used
    informationally (point 4 of the module docstring) to report the blitted emoji's
    measured ink aspect ratio. Ignores the magenta comparison-box outline (see
    `_is_comparison_box_color`) which is drawn around each emoji in the same region."""
    x0 = max(0, int(x0))
    x1 = min(img.width, int(x1))
    y0 = max(0, int(y0))
    y1 = min(img.height, int(y1))
    if x1 <= x0 or y1 <= y0:
        return None
    region = img.crop((x0, y0, x1, y1)).convert("RGB")
    px = region.load()
    left = right = top = bottom = None
    for y in range(region.height):
        for x in range(region.width):
            r, g, b = px[x, y]
            if (r < alpha_thresh or g < alpha_thresh or b < alpha_thresh) and not _is_comparison_box_color(r, g, b):
                left = x if left is None else min(left, x)
                right = x if right is None else max(right, x)
                top = y if top is None else min(top, y)
                bottom = y if bottom is None else max(bottom, y)
    if left is None:
        return None
    return (left + x0, top + y0, right + x0, bottom + y0)


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
        status_text, metrics, noto_ref, png_bytes, canvas_rect, cmp_box_info = attempt(cd_base, http_port, headless=True)
        mode = "headless"
        if not (metrics and metrics.get("emoji") and noto_ref):
            print("headless capture failed or inconclusive; retrying headed…")
            status_text, metrics, noto_ref, png_bytes, canvas_rect, cmp_box_info = attempt(
                cd_base, http_port, headless=False
            )
            mode = "headed"
    finally:
        chromedriver_proc.terminate()
        try:
            chromedriver_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            chromedriver_proc.kill()
        httpd.shutdown()

    screenshot_path = ARTIFACTS_DIR / "phase_four.png"
    screenshot_path.write_bytes(png_bytes)

    print(f"mode: {mode}")
    print(f"status: {status_text}")
    print(f"screenshot: {screenshot_path}")
    print("metrics (from the Rust/wasm app, #data-phase-four):")
    print(json.dumps(metrics, indent=2) if metrics else "  (none captured)")
    print("noto-ref (in-page real-Noto reference measurement, #data-noto-ref):")
    print(json.dumps(noto_ref, indent=2) if noto_ref else "  (none captured)")
    print("canvas_rect:", canvas_rect)

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
        # 1 & 2. Per-emoji advance parity.
        for e in metrics["emoji"]:
            ref_w = noto_ref["per_emoji"].get(e["char"])
            if ref_w is None:
                print(f"WARNING: no NotoRef reference measurement for {e['char']!r}")
                passed = False
                continue
            diff = abs(ref_w - e["noto_advance_px"])
            print(
                f"  {e['char']!r}: table={e['noto_advance_px']:.3f} NotoRef={ref_w:.3f} "
                f"apple={e['apple_advance_width']:.3f} "
                f"capture_font_size={e.get('capture_font_size', float('nan')):.3f} diff={diff:.3f}"
            )
            if diff > TABLE_VS_NOTOREF_TOL:
                print(f"WARNING: table-vs-NotoRef advance mismatch for {e['char']!r}: diff={diff:.3f}")
                passed = False

        sum_diff = abs(metrics["emoji_advance_sum"] - noto_ref["emoji_advance_sum"])
        print(
            f"  emoji_advance_sum: parley={metrics['emoji_advance_sum']:.3f} "
            f"noto_ref={noto_ref['emoji_advance_sum']:.3f} diff={sum_diff:.3f}"
        )
        if sum_diff > EMOJI_SUM_PARITY_TOL:
            print(f"WARNING: emoji-advance-sum parity failed: diff={sum_diff:.3f}")
            passed = False

        line_diff = abs(metrics["layout_width"] - noto_ref["line_width"])
        print(
            f"  line_width: parley={metrics['layout_width']:.3f} "
            f"noto_ref={noto_ref['line_width']:.3f} diff={line_diff:.3f}"
        )
        if line_diff > LINE_WIDTH_PARITY_TOL:
            print(f"NOTE: line-width parity (secondary) exceeded tolerance: diff={line_diff:.3f}")

        # 3. Baseline placement (phase-four acceptance metric, user ruling option A):
        # each emoji's own alphabetic baseline is blitted onto the shared Parley
        # baseline *plus* a fixed global downward shift `v_shift_px`, so
        # `e["baseline"]` (exported per-emoji, the actual blitted baseline) must equal
        # `metrics["line_baseline"] + metrics["v_shift_px"]` exactly — all three come
        # straight from the app's own `#data-phase-four` JSON, so this is a
        # same-source equality check (the shift is intentional and verified, not
        # drift), not a pixel-measurement comparison.
        line_baseline = metrics.get("line_baseline")
        v_shift_px = metrics.get("v_shift_px")
        if line_baseline is None or v_shift_px is None:
            print("WARNING: no line_baseline/v_shift_px in app metrics JSON; cannot check baseline placement")
            passed = False
        else:
            expected_emoji_baseline = line_baseline + v_shift_px
            print(
                f"  expected emoji_baseline = line_baseline + v_shift_px = "
                f"{line_baseline:.3f} + {v_shift_px:.3f} = {expected_emoji_baseline:.3f}"
            )
            for e in metrics["emoji"]:
                diff = abs(e["baseline"] - expected_emoji_baseline)
                print(
                    f"  {e['char']!r} baseline: emoji={e['baseline']:.3f} "
                    f"expected={expected_emoji_baseline:.3f} diff={diff:.3f}"
                )
                if diff > BASELINE_EXACT_TOL_PX:
                    print(f"WARNING: baseline placement failed for {e['char']!r} (tol={BASELINE_EXACT_TOL_PX}px)")
                    passed = False

        # 3b. HTML-overlay `.cmp-box` placement (rows 1/3): each row's boxes must sit at
        # `baseline - BOX_ASCENT` (top) and at the corresponding emoji's pen-left, in that
        # row's own local coordinate space — not shifted up into the label text above (the
        # historical bug: box offsets computed relative to the text line's own rect but
        # rendered relative to the outer `.row` container's positioned-ancestor origin).
        print("HTML overlay comparison-box placement (rows 1/3):")
        if not cmp_box_info:
            print("WARNING: no cmp-box measurement data collected")
            passed = False
        for row_info in cmp_box_info:
            row_id = row_info["rowId"]
            expected_top = row_info["boxAscentTarget"]
            pen_lefts = row_info["penLefts"]
            boxes = row_info["boxes"]
            if len(boxes) != len(pen_lefts):
                print(
                    f"  WARNING: row {row_id!r} has {len(boxes)} cmp-boxes but "
                    f"{len(pen_lefts)} emoji pen-lefts"
                )
                passed = False
            for i, box in enumerate(boxes):
                top_diff = abs(box["top"] - expected_top)
                expected_left = pen_lefts[i] if i < len(pen_lefts) else None
                left_diff = abs(box["left"] - expected_left) if expected_left is not None else None
                print(
                    f"  row={row_id!r} box[{i}]: top={box['top']:.3f} expected={expected_top:.3f} "
                    f"diff={top_diff:.3f}; left={box['left']:.3f} "
                    f"expected={expected_left if expected_left is None else round(expected_left, 3)} "
                    f"diff={left_diff if left_diff is None else round(left_diff, 3)}"
                )
                if top_diff > CMP_BOX_TOL_PX:
                    print(f"WARNING: row {row_id!r} box[{i}] top off baseline by {top_diff:.3f}px")
                    passed = False
                if left_diff is not None and left_diff > CMP_BOX_TOL_PX:
                    print(f"WARNING: row {row_id!r} box[{i}] left off pen-left by {left_diff:.3f}px")
                    passed = False

        # 4. Informational: measured blitted ink size (screenshot, alpha-threshold) vs
        # the app's own in-wasm pixel-measured ink size of the Apple capture
        # (`pixel_ink_width`/`pixel_ink_height`, from a per-pixel scan of the rasterized
        # capture — see capture.rs::pixel_ink_bbox). Not an acceptance criterion; lets a
        # human sanity-check that the blit didn't crop/clip the capture and that ink size
        # tracks `APPLE_TO_NOTO_EM_SCALE` as expected (global-scale ruling).
        if Image is None:
            print("WARNING: Pillow not available; skipping ink-size report.")
        else:
            img = Image.open(io.BytesIO(png_bytes))
            dpr = canvas_rect.get("dpr", 1) or 1
            canvas_left = canvas_rect.get("left", 0)
            canvas_top = canvas_rect.get("top", 0)
            canvas_top_dev = canvas_top * dpr
            canvas_bottom_dev = (canvas_top + canvas_rect.get("height", 1000)) * dpr
            for e in metrics["emoji"]:
                x0 = (canvas_left + e["x"]) * dpr
                x1 = x0 + e["noto_advance_px"] * dpr
                baseline_dev = (canvas_top + e["baseline"]) * dpr
                search_top = max(baseline_dev - 100 * dpr, canvas_top_dev)
                search_bottom = min(baseline_dev + 100 * dpr, canvas_bottom_dev)
                bbox = ink_bbox_in_region(img, x0, x1, search_top, search_bottom)
                if bbox is None:
                    print(f"  {e['char']!r} ink-size: no ink found in screenshot region")
                    continue
                left, top, right, bottom = bbox
                blit_w = (right - left + 1) / dpr
                blit_h = (bottom - top + 1) / dpr
                capture_w = e["pixel_ink_width"]
                capture_h = e["pixel_ink_height"]
                print(
                    f"  {e['char']!r} ink-size: blitted(screenshot)={blit_w:.2f}x{blit_h:.2f} "
                    f"capture(pixel_ink_width/height)={capture_w:.2f}x{capture_h:.2f} "
                    f"capture_font_size={e.get('capture_font_size', float('nan')):.3f}"
                )

    # 4. Payload audit.
    print()
    print("dist/ payload audit:")
    apple_font_found = False
    full_noto_found = False
    for p in sorted(DIST_DIR.rglob("*")):
        if p.is_file():
            size_kib = p.stat().st_size / 1024
            print(f"  {p.relative_to(DIST_DIR)}: {size_kib:.1f} KiB")
            name_lower = p.name.lower()
            is_font_file = p.suffix.lower() in (".ttf", ".otf", ".woff", ".woff2", ".ttc")
            if is_font_file and ("apple" in name_lower or "sfpro" in name_lower or "sf-pro" in name_lower):
                apple_font_found = True
            if is_font_file and "notocoloremoji" in name_lower.replace(" ", "").replace("_", "") and size_kib > 150:
                full_noto_found = True
    if apple_font_found:
        print("WARNING: dist/ appears to contain an Apple font!")
        passed = False
    if full_noto_found:
        print("WARNING: dist/ appears to contain the full Noto Color Emoji font!")
        passed = False

    print()
    print("RESULT:", "PASS" if passed else "FAIL")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
