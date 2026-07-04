#!/usr/bin/env python3
# Copyright 2026 the Parley Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""ChromeDriver-driven Noto Color Emoji ink survey + combined Apple/Noto analysis.

Serves index_noto.html and the real NotoColorEmoji-Regular.ttf (on /noto-ref),
drives Chrome for Testing headless, scrapes measured Noto pixel-ink for the same
emoji ranges as the Apple survey, saves noto_results.json, then loads the existing
Apple results.json and writes noto_analysis.txt answering the global-scale question.

Usage:
    cd examples/apple_emoji/ink_survey
    python3 survey_noto.py
"""

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
REPO = HERE.parents[2]
NOTO_TTF = REPO / "parley_dev/assets/fonts/noto_color_emoji/NotoColorEmoji-Regular.ttf"
CHROMEDRIVER_PORT = 9515
HTTP_PORT = 8189

# Noto Color Emoji uniform advance @48px (1275/1024 em); matches phase_three.
NOTO_ADV_48 = 1275 / 1024 * 48   # 59.766
NOTO_ADV_128 = 1275 / 1024 * 128  # 159.375


def find_free_port_or(default):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind(("127.0.0.1", default))
            return default
        except OSError:
            s.bind(("127.0.0.1", 0))
            return s.getsockname()[1]


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(HERE), **kwargs)

    def log_message(self, fmt, *args):
        pass

    def do_GET(self):
        # Measurement-only route: serve the real Noto Color Emoji font.
        if self.path == "/noto-ref":
            data = NOTO_TTF.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "font/ttf")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(data)
            return
        # Serve index_noto.html at root.
        if self.path == "/":
            self.path = "/index_noto.html"
        return super().do_GET()


def serve_dir(port):
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
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
    deadline = time.time() + 240
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


def cv(vals):
    vals = [v for v in vals if v is not None]
    if len(vals) < 2:
        return float("nan")
    m = statistics.mean(vals)
    return statistics.pstdev(vals) / abs(m) if m else float("nan")


def describe(name, vals, unit=""):
    vals = [v for v in vals if v is not None]
    if not vals:
        return f"{name}: (no data)"
    return (f"{name}: n={len(vals)} mean={statistics.mean(vals):.3f}{unit} "
            f"median={statistics.median(vals):.3f}{unit} "
            f"p5={pct(vals, 5):.3f} p95={pct(vals, 95):.3f} "
            f"min={min(vals):.3f} max={max(vals):.3f} CV={cv(vals):.3f}")


def analyze(noto, apple):
    lines = []
    L = lines.append

    noto_rows = {r["cp"]: r for r in noto["results"]}
    apple_rows = {r["cp"]: r for r in apple["results"]}

    L("=" * 64)
    L("NOTO COLOR EMOJI ink survey (real NotoColorEmoji-Regular.ttf)")
    L("=" * 64)
    L(f"font served: {NOTO_TTF.name}")
    L(f"fontLoaded flag (document.fonts.check): {noto.get('fontLoaded')}")
    L(f"Noto candidates: {noto['candidateCount']}  kept(color): {noto['keptCount']}  "
      f"skipped non-color(tofu/uncovered): {noto['skippedNonColor']}  unrendered: {noto['notRendered']}")
    L(f"Apple kept(color): {apple['keptCount']}")
    L(f"Uniform Noto advance @48px = {NOTO_ADV_48:.3f}px, @128px = {NOTO_ADV_128:.3f}px")
    L("")

    # ---- Sanity: Noto really differs from Apple (not a fallback) ----
    both_cp = sorted(set(noto_rows) & set(apple_rows))
    diff_w = same_w = 0
    for cp in both_cp:
        n = noto_rows[cp].get("m48")
        a = apple_rows[cp].get("m48")
        if n and a:
            if n["measInkW"] != a["measInkW"]:
                diff_w += 1
            else:
                same_w += 1
    L(f"SANITY: emoji in both Noto & Apple sets: {len(both_cp)}")
    L(f"  Noto ink width != Apple ink width for {diff_w}/{diff_w+same_w} "
      f"({100*diff_w/max(1,diff_w+same_w):.1f}%) -> confirms Noto rendered, not Apple fallback")
    L("")

    # ---- Q1: Noto ink width @48 distribution + as fraction of advance ----
    for size_key, size, adv in (("m48", 48, NOTO_ADV_48), ("m128", 128, NOTO_ADV_128)):
        rows = [r[size_key] for r in noto["results"] if r.get(size_key)]
        L(f"----- NOTO @{size}px (n={len(rows)}) -----")
        ink_w = [m["measInkW"] for m in rows]
        ink_h = [m["measInkH"] for m in rows]
        frac_w = [m["measInkW"] / adv for m in rows]
        frac_h = [m["measInkH"] / adv for m in rows]
        asc = [m["measAscent"] for m in rows]     # ink above baseline
        desc = [m["measDescent"] for m in rows]   # ink below baseline (dip)
        left = [m["measLeft"] for m in rows]       # ink extent left of advance-box left edge
        center = [m["centerOffset"] for m in rows]
        # horizontal margin: advance - ink width (total slack), and per-side.
        margin = [adv - m["measInkW"] for m in rows]
        L("  " + describe("ink width px", ink_w))
        L("  " + describe("ink width / advance", frac_w))
        L("  " + describe("horizontal margin px (advance-inkW)", margin))
        L("  " + describe("ink height px", ink_h))
        L("  " + describe("ink height / advance", frac_h))
        L("  " + describe("ink ascent px (above baseline)", asc))
        L("  " + describe("ink descent px (below baseline, dip)", desc))
        L("  " + describe("ink descent / advance", [d / adv for d in desc]))
        L("  " + describe("ink left offset px (from box left)", left))
        L("  " + describe("center offset px (ink center vs advance center)", center))
        L("")

    # ---- Q3: Apple ink width @48 as fraction of 48px em ----
    for size_key, size in (("m48", 48), ("m128", 128)):
        rows = [r[size_key] for r in apple["results"] if r.get(size_key)]
        L(f"----- APPLE @{size}px (n={len(rows)}) -----")
        ink_w = [m["measInkW"] for m in rows]
        ink_h = [m["measInkH"] for m in rows]
        L("  " + describe("ink width px", ink_w))
        L("  " + describe("ink width / em (=fontSize)", [m["measInkW"] / size for m in rows]))
        L("  " + describe("ink height px", ink_h))
        L("  " + describe("ink height / em", [m["measInkH"] / size for m in rows]))
        L("  " + describe("ink descent px (below baseline)", [m["measDescent"] for m in rows]))
        L("")

    # ---- Q4: per-emoji noto_ink / apple_ink, and recommended global scale ----
    # Phases render Apple emoji at em=fontSize into the Noto advance box. To make the
    # Apple ink occupy the same fraction of the Noto box that Noto ink does, scale the
    # Apple em by (noto_inkW / apple_inkW). That per-emoji ratio's spread tells us how
    # well a SINGLE global factor works.
    L("=" * 64)
    L("Q4: per-emoji Noto-ink / Apple-ink ratio  ==> GLOBAL SCALE candidate")
    L("=" * 64)
    ratios_w = []
    ratios_h = []
    # Also the simpler model actually used by phases: scale Apple so its ink width
    # matches Noto's ink width, i.e. new_em = 48 * (noto_inkW / apple_inkW).
    for cp in both_cp:
        n = noto_rows[cp].get("m48")
        a = apple_rows[cp].get("m48")
        if n and a and a["measInkW"] > 0 and a["measInkH"] > 0:
            ratios_w.append(n["measInkW"] / a["measInkW"])
            ratios_h.append(n["measInkH"] / a["measInkH"])
    L("  " + describe("noto_inkW / apple_inkW (@48)", ratios_w))
    L("  " + describe("noto_inkH / apple_inkH (@48)", ratios_h))
    med_w = statistics.median(ratios_w)
    L("")
    L(f"  Recommended GLOBAL width-match scale (median ratio): {med_w:.4f}")
    L(f"    p5={pct(ratios_w,5):.4f}  p95={pct(ratios_w,95):.4f}  CV={cv(ratios_w):.4f}")

    # Alternative global model: single em fraction. Apple ink fills what fraction of
    # the Noto advance box on average? Target Noto's median inkW/advance.
    noto48 = [r["m48"] for r in noto["results"] if r.get("m48")]
    apple48 = [r["m48"] for r in apple["results"] if r.get("m48")]
    noto_fill = statistics.median([m["measInkW"] / NOTO_ADV_48 for m in noto48])
    apple_fill = statistics.median([m["measInkW"] / 48 for m in apple48])
    # To make Apple ink fill noto_fill of the Noto box: em_scale so that
    # (apple_fill * em_scale * 48) / NOTO_ADV_48 = noto_fill.
    global_em_scale = (noto_fill * NOTO_ADV_48) / (apple_fill * 48)
    L("")
    L(f"  Median Noto ink fills {noto_fill:.3f} of its advance box.")
    L(f"  Median Apple ink fills {apple_fill:.3f} of its 48px em.")
    L(f"  Global em-scale to match median fill: {global_em_scale:.4f}  "
      f"(Apple emoji drawn at em = {global_em_scale*48:.2f}px inside the {NOTO_ADV_48:.2f}px box)")

    # ---- Vertical offset recommendation ----
    noto_desc = statistics.median([m["measDescent"] for m in noto48])
    apple_desc = statistics.median([m["measDescent"] for m in apple48])
    noto_asc = statistics.median([m["measAscent"] for m in noto48])
    L("")
    L("  VERTICAL: median Noto ink descent below baseline = "
      f"{noto_desc:.2f}px ({noto_desc/NOTO_ADV_48*100:.1f}% of advance); "
      f"median ascent = {noto_asc:.2f}px")
    L(f"           median Apple ink descent below baseline = {apple_desc:.2f}px")

    # ---- Hero emoji ----
    L("")
    L("----- HERO EMOJI (@48px) -----")
    for cp, name in ((0x1F600, "😀 U+1F600"), (0x1F389, "🎉 U+1F389")):
        n = noto_rows.get(cp, {}).get("m48") if cp in noto_rows else None
        a = apple_rows.get(cp, {}).get("m48") if cp in apple_rows else None
        if n:
            L(f"  {name} NOTO : inkW={n['measInkW']} inkH={n['measInkH']} "
              f"(inkW/adv={n['measInkW']/NOTO_ADV_48:.3f}) asc={n['measAscent']} "
              f"desc={n['measDescent']} centerOff={n['centerOffset']:.2f}")
        else:
            L(f"  {name} NOTO : (not covered)")
        if a:
            L(f"  {name} APPLE: inkW={a['measInkW']} inkH={a['measInkH']} "
              f"(inkW/em={a['measInkW']/48:.3f}) asc={a['measAscent']} desc={a['measDescent']}")
        if n and a and a["measInkW"] > 0:
            L(f"  {name} ratio noto/apple inkW = {n['measInkW']/a['measInkW']:.4f}")
    return "\n".join(lines)


def main():
    if not NOTO_TTF.exists():
        print(f"ERROR: Noto font not found at {NOTO_TTF}", file=sys.stderr)
        return 1
    apple_path = HERE / "results.json"
    if not apple_path.exists():
        print("ERROR: results.json (Apple survey) not found; run survey.py first",
              file=sys.stderr)
        return 1
    apple = json.loads(apple_path.read_text())

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

    (HERE / "noto_results.json").write_text(json.dumps(data))
    summary = analyze(data, apple)
    (HERE / "noto_analysis.txt").write_text(summary)
    print()
    print(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
