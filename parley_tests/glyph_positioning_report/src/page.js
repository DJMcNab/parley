// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Two jobs: load the case's fonts with their bytes pinned, and keep the cluster
// highlight in step across the raster, the hex list and the diff table.
//
// `FACES` and `GEOMETRY` are emitted immediately above this script by `page.rs`.

/**
 * Fetches a face, verifies its SHA-256, and registers it.
 *
 * `@font-face` cannot carry an `integrity` attribute, so pinning means doing the fetch
 * by hand. If this is skipped or fails silently, the live-DOM panel renders in a
 * fallback face while the images above it were rasterised from the real one — which
 * reads as a Parley bug rather than as the font mismatch it is. So every failure path
 * here is loud.
 */
async function loadFace(face) {
  const response = await fetch(face.url);
  if (!response.ok) {
    throw new Error(`fetch failed: HTTP ${response.status}`);
  }
  const bytes = await response.arrayBuffer();
  const digest = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  if (digest !== face.sha256) {
    throw new Error(`SHA-256 mismatch\n  expected ${face.sha256}\n  got      ${digest}`);
  }
  const fontFace = new FontFace(face.family, bytes);
  await fontFace.load();
  document.fonts.add(fontFace);
}

async function loadFaces() {
  const errors = [];
  for (const face of FACES) {
    try {
      await loadFace(face);
    } catch (error) {
      errors.push(`${face.family} (${face.url})\n  ${error.message}`);
    }
  }
  if (errors.length !== 0) {
    const box = document.getElementById("font-error");
    box.hidden = false;
    box.textContent =
      "The case's font could not be loaded, so the live DOM panel and the character " +
      "samples below are rendering in a fallback face. The images above are unaffected " +
      "— they were rasterised from the local font bytes.\n\n" + errors.join("\n\n");
  }
}

/** Lights up every element belonging to `cluster` (or clears the highlight for null). */
function setActive(cluster) {
  for (const element of document.querySelectorAll("[data-cluster].active")) {
    element.classList.remove("active");
  }
  if (cluster === null) {
    return;
  }
  for (const element of document.querySelectorAll(`[data-cluster="${cluster}"]`)) {
    element.classList.add("active");
  }
}

document.addEventListener("mouseover", (event) => {
  const owner = event.target.closest("[data-cluster]");
  setActive(owner === null ? null : owner.dataset.cluster);
});

loadFaces();
