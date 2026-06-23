// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Posts (or updates in place) the single sticky Tango benchmark comment on a pull
// request. Invoked from `.github/workflows/benchmark-comment.yml` via
// `actions/github-script`, which passes in the authenticated `github` client,
// the `context`, and `core`.
//
// The comment is identified by a hidden HTML marker so repeated runs update the
// same comment rather than posting a new one each time.

const fs = require("fs");

// Hidden marker used to locate our own comment among all PR comments.
const MARKER = "<!-- tango-benchmarks -->";

module.exports = async ({ github, context, core }) => {
  const body = fs.readFileSync("pr-comment/comment.md", "utf8");
  const number = Number(fs.readFileSync("pr-comment/pr-number.txt", "utf8").trim());

  if (!Number.isInteger(number) || number <= 0) {
    core.setFailed(`Invalid PR number in artifact: ${JSON.stringify(number)}`);
    return;
  }

  const payload = `${MARKER}\n${body}`;

  // Paginate so we find the sticky comment even on PRs with many comments.
  const comments = await github.paginate(github.rest.issues.listComments, {
    ...context.repo,
    issue_number: number,
    per_page: 100,
  });
  const existing = comments.find((comment) => comment.body.includes(MARKER));

  if (existing) {
    core.info(`Updating existing benchmark comment ${existing.id} on PR #${number}.`);
    await github.rest.issues.updateComment({
      ...context.repo,
      comment_id: existing.id,
      body: payload,
    });
  } else {
    core.info(`Creating new benchmark comment on PR #${number}.`);
    await github.rest.issues.createComment({
      ...context.repo,
      issue_number: number,
      body: payload,
    });
  }
};
