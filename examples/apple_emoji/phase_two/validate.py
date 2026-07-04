#!/usr/bin/env python3
# Copyright 2026 the Parley Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Dependency-free ChromeDriver validation harness for Phase Two.

Copied/adapted from `examples/apple_emoji/phase_one/validate.py` (phase_one is not
edited or imported).

Serves the trunk `dist/` build over HTTP, drives Chrome for Testing via
ChromeDriver's W3C WebDriver protocol, waits for the page to report "done", scrapes the
per-emoji metrics JSON and takes a screenshot.

Usage:
    cd examples/apple_emoji/phase_two
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

CHROMEDRIVER_PORT = 9516
HTTP_PORT = 8178


def find_free_port_or(default):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind(("127.0.0.1", default))
            return default
        except OSError:
            s.bind(("127.0.0.1", 0))
            return s.getsockname()[1]


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(DIST_DIR), **kwargs)

    def log_message(self, fmt, *args):
        pass


def serve_dist(port):
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), QuietHandler)
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

    data_result = wd_request(
        base,
        "POST",
        f"/session/{session_id}/execute/sync",
        {
            "script": "return document.getElementById('data-phase-two').textContent;",
            "args": [],
        },
    )
    data_json_text = data_result.get("value") or ""

    metrics = None
    if data_json_text:
        try:
            metrics = json.loads(data_json_text)
        except json.JSONDecodeError:
            metrics = None

    # Cross-measure the native reference for a consistency check: same font stack,
    # same size, measured in the page itself.
    native_check = wd_request(
        base,
        "POST",
        f"/session/{session_id}/execute/sync",
        {
            "script": """
                const c = document.createElement('canvas');
                const ctx = c.getContext('2d');
                ctx.font = "48px 'Apple Color Emoji', 'Segoe UI Emoji', 'Noto Color Emoji', sans-serif";
                const out = {};
                for (const ch of ['\\u{1F600}', '\\u{1F389}']) {
                    out[ch] = ctx.measureText(ch).width;
                }
                return out;
            """,
            "args": [],
        },
    )

    screenshot_result = wd_request(base, "GET", f"/session/{session_id}/screenshot")
    png_bytes = base64.b64decode(screenshot_result["value"])

    return status_text, metrics, native_check.get("value"), png_bytes


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


def main():
    if not DIST_DIR.exists():
        print(f"error: {DIST_DIR} does not exist; run `trunk build` first.", file=sys.stderr)
        return 1

    ARTIFACTS_DIR.mkdir(exist_ok=True)

    http_port = find_free_port_or(HTTP_PORT)
    httpd = serve_dist(http_port)

    chromedriver_proc, cd_base = start_chromedriver()
    try:
        status_text, metrics, native_check, png_bytes = attempt(cd_base, http_port, headless=True)
        mode = "headless"
        if not (metrics and metrics.get("emoji")):
            print("headless capture failed or inconclusive; retrying headed…")
            status_text, metrics, native_check, png_bytes = attempt(cd_base, http_port, headless=False)
            mode = "headed"
    finally:
        chromedriver_proc.terminate()
        try:
            chromedriver_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            chromedriver_proc.kill()
        httpd.shutdown()

    screenshot_path = ARTIFACTS_DIR / "phase_two.png"
    screenshot_path.write_bytes(png_bytes)

    print(f"mode: {mode}")
    print(f"status: {status_text}")
    print(f"screenshot: {screenshot_path}")
    print("metrics:")
    print(json.dumps(metrics, indent=2) if metrics else "  (none captured)")
    print("native cross-check (measureText for same emoji/font-size in-page):")
    print(json.dumps(native_check, indent=2) if native_check else "  (none captured)")

    passed = bool(status_text.startswith("done") and metrics and metrics.get("emoji"))
    if passed and native_check:
        for e in metrics["emoji"]:
            native_w = native_check.get(e["char"])
            if native_w is not None and abs(native_w - e["advance_width"]) > 0.001:
                print(f"WARNING: advance mismatch for {e['char']!r}: box={e['advance_width']} native={native_w}")
                passed = False

    print()
    print("RESULT:", "PASS" if passed else "FAIL")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
