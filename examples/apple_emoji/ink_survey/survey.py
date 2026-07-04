#!/usr/bin/env python3
# Copyright 2026 the Parley Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""ChromeDriver-driven Apple emoji ink survey.

Serves index.html, drives Chrome for Testing headless (falls back to headed if the
color check fails), waits for the page to finish the survey, scrapes the #data JSON,
saves it to results.json and prints distribution statistics.

Usage:
    cd examples/apple_emoji/ink_survey
    python3 survey.py
"""

import base64
import http.server
import json
import socket
import statistics
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

CHROMEDRIVER = "/Users/djmcnab/chromedriver/mac_arm-150.0.7871.46/chromedriver-mac-arm64/chromedriver"
CHROME_BINARY = (
    "/Users/djmcnab/chrome/mac_arm-150.0.7871.46/chrome-mac-arm64/"
    "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
)

HERE = Path(__file__).resolve().parent
CHROMEDRIVER_PORT = 9515
HTTP_PORT = 8188


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
        super().__init__(*args, directory=str(HERE), **kwargs)

    def log_message(self, fmt, *args):
        pass


def serve_dir(port):
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), QuietHandler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def wd(base, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"{base}{path}", data=data,
        headers={"Content-Type": "application/json"}, method=method,
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read().decode())


def start_chromedriver():
    proc = subprocess.Popen(
        [CHROMEDRIVER, f"--port={CHROMEDRIVER_PORT}"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    base = f"http://127.0.0.1:{CHROMEDRIVER_PORT}"
    for _ in range(50):
        try:
            urllib.request.urlopen(f"{base}/status", timeout=1)
            return proc, base
        except Exception:
            time.sleep(0.1)
    raise RuntimeError("chromedriver did not start")


def new_session(base, headless):
    args = ["--window-size=900,1100", "--force-color-profile=srgb"]
    if headless:
        args.insert(0, "--headless=new")
    payload = {"capabilities": {"alwaysMatch": {
        "goog:chromeOptions": {"binary": CHROME_BINARY, "args": args}}}}
    return wd(base, "POST", "/session", payload)["value"]["sessionId"]


def run_session(base, sid, http_port):
    wd(base, "POST", f"/session/{sid}/url", {"url": f"http://127.0.0.1:{http_port}"})
    deadline = time.time() + 180
    status = ""
    while time.time() < deadline:
        status = wd(base, "POST", f"/session/{sid}/execute/sync",
                    {"script": "return document.getElementById('status').textContent;",
                     "args": []}).get("value") or ""
        if status.startswith("done"):
            break
        time.sleep(0.5)
    data_text = wd(base, "POST", f"/session/{sid}/execute/sync",
                   {"script": "return document.getElementById('data').textContent;",
                    "args": []}).get("value") or ""
    data = json.loads(data_text) if data_text else None
    return status, data


def attempt(base, http_port, headless):
    sid = new_session(base, headless)
    try:
        return run_session(base, sid, http_port)
    finally:
        try:
            wd(base, "DELETE", f"/session/{sid}")
        except Exception:
            pass


def pct(vals, p):
    if not vals:
        return float("nan")
    s = sorted(vals)
    k = (len(s) - 1) * (p / 100.0)
    lo = int(k)
    hi = min(lo + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def describe(name, vals):
    vals = [v for v in vals if v is not None]
    if not vals:
        return f"{name}: (no data)"
    return (f"{name}: n={len(vals)} mean={statistics.mean(vals):.3f} "
            f"median={statistics.median(vals):.3f} "
            f"p5={pct(vals, 5):.3f} p95={pct(vals, 95):.3f} "
            f"min={min(vals):.3f} max={max(vals):.3f}")


def analyze(data):
    results = data["results"]
    lines = []
    lines.append(f"Candidates generated: {data['candidateCount']}")
    lines.append(f"Kept (color emoji):   {data['keptCount']}")
    lines.append(f"Skipped non-color:    {data['skippedNonColor']}")
    lines.append(f"Not rendered:         {data['notRendered']}")
    lines.append("")

    for size_key, size in (("m48", 48), ("m128", 128)):
        rows = [r[size_key] for r in results if r.get(size_key)]
        lines.append(f"===== Font size {size}px  (n={len(rows)}) =====")
        # Width: provided ink vs measured ink.
        w_ratio = [m["provInkW"] / m["measInkW"] for m in rows if m["measInkW"] > 0]
        w_delta = [m["provInkW"] - m["measInkW"] for m in rows]
        h_ratio = [m["provInkH"] / m["measInkH"] for m in rows if m["measInkH"] > 0]
        h_delta = [m["provInkH"] - m["measInkH"] for m in rows]
        asc_delta = [m["provAscent"] - m["measAscent"] for m in rows]
        desc_delta = [m["provDescent"] - m["measDescent"] for m in rows]
        adv_ratio = [m["advance"] / m["measInkW"] for m in rows if m["measInkW"] > 0]
        center = [m["centerOffset"] for m in rows]
        # Fraction of provided ink width the measured falls short: delta / provided.
        w_frac = [(m["provInkW"] - m["measInkW"]) / m["provInkW"]
                  for m in rows if m["provInkW"] > 0]
        h_frac = [(m["provInkH"] - m["measInkH"]) / m["provInkH"]
                  for m in rows if m["provInkH"] > 0]
        clipped = sum(1 for m in rows if m.get("borderClipped"))

        lines.append("  -- WIDTH (provided ink vs measured pixel ink) --")
        lines.append("  " + describe("provInkW/measInkW ratio", w_ratio))
        lines.append("  " + describe("provInkW-measInkW px delta", w_delta))
        lines.append("  " + describe("(prov-meas)/prov width fraction", w_frac))
        lines.append("  -- HEIGHT --")
        lines.append("  " + describe("provInkH/measInkH ratio", h_ratio))
        lines.append("  " + describe("provInkH-measInkH px delta", h_delta))
        lines.append("  " + describe("(prov-meas)/prov height fraction", h_frac))
        lines.append("  -- ASCENT / DESCENT px delta (provided-measured) --")
        lines.append("  " + describe("ascent delta", asc_delta))
        lines.append("  " + describe("descent delta", desc_delta))
        lines.append("  -- ADVANCE --")
        lines.append("  " + describe("advance/measInkW ratio", adv_ratio))
        lines.append("  " + describe("advance px", [m["advance"] for m in rows]))
        lines.append("  -- CENTER OFFSET (measured ink center vs advance center, px) --")
        lines.append("  " + describe("centerOffset", center))
        lines.append(f"  border-clipped rows: {clipped}")

        # Constant vs proportional test: is delta better modeled as constant px or
        # constant fraction of size? Compare coefficient of variation.
        if w_delta:
            cv_delta = statistics.pstdev(w_delta) / abs(statistics.mean(w_delta)) if statistics.mean(w_delta) else float('nan')
            cv_frac = statistics.pstdev(w_frac) / abs(statistics.mean(w_frac)) if statistics.mean(w_frac) else float('nan')
            lines.append(f"  width delta CV (px-constant model):   {cv_delta:.3f}")
            lines.append(f"  width frac  CV (proportional model):  {cv_frac:.3f}")
        lines.append("")

    # Specific hero emoji.
    for cp, name in ((0x1F600, "😀 U+1F600"), (0x1F389, "🎉 U+1F389")):
        r = next((r for r in results if r["cp"] == cp), None)
        lines.append(f"----- {name} -----")
        if not r:
            lines.append("  (not in kept set)")
            continue
        for size_key, size in (("m48", 48), ("m128", 128)):
            m = r.get(size_key)
            if not m:
                continue
            lines.append(f"  {size}px: advance={m['advance']:.2f} "
                         f"provInkW={m['provInkW']:.2f} measInkW={m['measInkW']} "
                         f"(delta={m['provInkW']-m['measInkW']:.2f}, "
                         f"frac={(m['provInkW']-m['measInkW'])/m['provInkW']*100:.1f}%)  "
                         f"provInkH={m['provInkH']:.2f} measInkH={m['measInkH']} "
                         f"centerOff={m['centerOffset']:.2f}")
    return "\n".join(lines)


def main():
    http_port = find_free_port_or(HTTP_PORT)
    httpd = serve_dir(http_port)
    proc, cd_base = start_chromedriver()
    try:
        status, data = attempt(cd_base, http_port, headless=True)
        mode = "headless"
        ok = data and data.get("keptCount", 0) > 0
        if not ok:
            print("headless produced no color emoji; retrying headed…")
            status, data = attempt(cd_base, http_port, headless=False)
            mode = "headed"
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        httpd.shutdown()

    print(f"mode: {mode}")
    print(f"status: {status}")
    if not data:
        print("ERROR: no data captured", file=sys.stderr)
        return 1

    (HERE / "results.json").write_text(json.dumps(data))
    summary = analyze(data)
    (HERE / "summary.txt").write_text(summary)
    print()
    print(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
