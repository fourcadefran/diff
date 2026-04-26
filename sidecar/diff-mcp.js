#!/usr/bin/env node

import { randomUUID } from "node:crypto";
import fs from "node:fs/promises";
import { readFileSync, rmSync } from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import Database from "better-sqlite3";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod/v4";

const STATE_DIR = path.join(os.homedir(), ".diff");
const STATE_PATH = path.join(STATE_DIR, "review-bridge.json");
const DB_PATH = path.join(STATE_DIR, "reviews.db");

const reviewCommentSchema = z.object({
  key: z.string().min(1),
  file_path: z.string().min(1),
  line_start: z.number().int().nonnegative(),
  line_end: z.number().int().nonnegative(),
  comment: z.string().min(1),
  action_type: z.enum(["change-request", "question", "nit"]),
});

const reviewItemOutputSchema = {
  id: z.string(),
  key: z.string(),
  file_path: z.string(),
  line_start: z.number().int().nonnegative(),
  line_end: z.number().int().nonnegative(),
  comment: z.string(),
  action_type: z.enum(["change-request", "question", "nit"]),
  status: z.enum(["pending", "acknowledged", "resolved", "dismissed"]),
  summary: z.string().nullable(),
  dismiss_reason: z.string().nullable(),
};

const reviewBatchOutputSchema = {
  message: z.string(),
  review: z.object({
    id: z.string(),
    status: z.enum(["pending", "in_progress", "resolved"]),
    reviews: z.array(z.object(reviewItemOutputSchema)),
    created_at: z.string(),
    updated_at: z.string(),
    resolved_at: z.string().nullable(),
  }).nullable(),
};

// ── Shared helpers ──────────────────────────────────────────────────

function readJson(body) {
  return new Promise((resolve, reject) => {
    let data = "";
    body.setEncoding("utf8");
    body.on("data", (chunk) => (data += chunk));
    body.on("end", () => {
      try {
        resolve(JSON.parse(data));
      } catch {
        reject(new Error("invalid JSON body"));
      }
    });
    body.on("error", reject);
  });
}

function sendJson(res, status, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  res.end(body);
}

async function readServerInfo() {
  try {
    const raw = await fs.readFile(STATE_PATH, "utf8");
    return JSON.parse(raw);
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

async function writeServerInfo(info) {
  await fs.mkdir(STATE_DIR, { recursive: true });
  await fs.writeFile(STATE_PATH, JSON.stringify(info), "utf8");
}

async function removeServerInfoIfOwned(pid) {
  const info = await readServerInfo().catch(() => null);
  if (info?.pid === pid) {
    await fs.rm(STATE_PATH, { force: true });
  }
}

// ── Database ────────────────────────────────────────────────────────

function initDatabase() {
  const db = new Database(DB_PATH);
  db.pragma("journal_mode = WAL");
  db.exec(`
    CREATE TABLE IF NOT EXISTS reviews (
      id          TEXT PRIMARY KEY,
      status      TEXT NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'in_progress', 'resolved')),
      created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
      updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
      resolved_at TEXT
    )
  `);
  db.exec(`
    CREATE TABLE IF NOT EXISTS comments (
      id             TEXT PRIMARY KEY,
      review_id      TEXT NOT NULL REFERENCES reviews(id),
      key            TEXT NOT NULL,
      file_path      TEXT NOT NULL,
      line_start     INTEGER NOT NULL,
      line_end       INTEGER NOT NULL,
      comment        TEXT NOT NULL,
      action_type    TEXT NOT NULL CHECK (action_type IN ('change-request', 'question', 'nit')),
      status         TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'acknowledged', 'resolved', 'dismissed')),
      summary        TEXT,
      dismiss_reason TEXT,
      created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
      updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )
  `);
  return db;
}

// ── SSE ─────────────────────────────────────────────────────────────

/** @type {Set<http.ServerResponse>} */
const sseClients = new Set();

function broadcastSSE(event, data) {
  const payload = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
  for (const client of sseClients) {
    try {
      client.write(payload);
    } catch {
      sseClients.delete(client);
    }
  }
}

// ── HTTP Server mode ────────────────────────────────────────────────

async function startServer() {
  await fs.mkdir(STATE_DIR, { recursive: true });
  const db = initDatabase();

  // ── Prepared statements ───────────────────────────────────────────

  const insertReview = db.prepare(
    "INSERT INTO reviews (id) VALUES (?)",
  );
  const insertComment = db.prepare(`
    INSERT INTO comments (id, review_id, key, file_path, line_start, line_end, comment, action_type)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
  `);
  const getPendingReview = db.prepare(
    "SELECT * FROM reviews WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1",
  );
  const getReviewById = db.prepare(
    "SELECT * FROM reviews WHERE id = ?",
  );
  const getCommentsByReview = db.prepare(
    "SELECT * FROM comments WHERE review_id = ? ORDER BY created_at ASC",
  );
  const getCommentById = db.prepare(
    "SELECT * FROM comments WHERE id = ?",
  );
  const markReviewInProgress = db.prepare(
    "UPDATE reviews SET status = 'in_progress', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
  );
  const markReviewResolved = db.prepare(
    "UPDATE reviews SET status = 'resolved', resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
  );
  const acknowledgeComment = db.prepare(
    "UPDATE comments SET status = 'acknowledged', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND status IN ('pending')",
  );
  const resolveComment = db.prepare(
    "UPDATE comments SET status = 'resolved', summary = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND status IN ('pending', 'acknowledged')",
  );
  const dismissComment = db.prepare(
    "UPDATE comments SET status = 'dismissed', dismiss_reason = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND status IN ('pending', 'acknowledged')",
  );
  const countOpenComments = db.prepare(
    "SELECT COUNT(*) as cnt FROM comments WHERE review_id = ? AND status IN ('pending', 'acknowledged')",
  );

  // ── Transactions ──────────────────────────────────────────────────

  const createReview = db.transaction((comments) => {
    const reviewId = randomUUID();
    insertReview.run(reviewId);
    const commentIds = [];
    for (const c of comments) {
      const commentId = randomUUID();
      insertComment.run(
        commentId, reviewId, c.key, c.file_path,
        c.line_start, c.line_end, c.comment, c.action_type,
      );
      commentIds.push({ key: c.key, id: commentId });
    }
    return { reviewId, commentIds };
  });

  const claimPending = db.transaction(() => {
    const row = getPendingReview.get();
    if (!row) return null;
    markReviewInProgress.run(row.id);
    return row;
  });

  /** Auto-resolve review if all comments are in terminal state */
  function autoResolveReview(reviewId) {
    const { cnt } = countOpenComments.get(reviewId);
    if (cnt === 0) {
      const review = getReviewById.get(reviewId);
      if (review && review.status !== "resolved") {
        markReviewResolved.run(reviewId);
      }
    }
  }

  function expandReview(row) {
    const reviews = getCommentsByReview.all(row.id).map((c) => ({
      id: c.id,
      key: c.key,
      file_path: c.file_path,
      line_start: c.line_start,
      line_end: c.line_end,
      comment: c.comment,
      action_type: c.action_type,
      status: c.status,
      summary: c.summary,
      dismiss_reason: c.dismiss_reason,
    }));
    return {
      id: row.id,
      status: row.status,
      reviews,
      created_at: row.created_at,
      updated_at: row.updated_at,
      resolved_at: row.resolved_at,
    };
  }

  function formatReviewSummary(review, headline) {
    const instructions = review.reviews
      .map((item, i) => {
        const loc = item.line_start === item.line_end
          ? `line ${item.line_start}`
          : `lines ${item.line_start}-${item.line_end}`;
        const guidance = item.action_type === "question"
          ? "Answer the question in resolve_review.summary. Do not change code unless the review explicitly asks for a code change."
          : item.action_type === "nit"
            ? "Make a minor improvement if useful, or dismiss_review with a reason."
            : "Apply the requested code change.";
        return `${i + 1}. [${item.action_type}] ${item.file_path} (${loc}): ${item.comment}\n   review_id: ${item.id}\n   guidance: ${guidance}`;
      })
      .join("\n");

    return (
      `${headline} (id: ${review.id}) — ${review.reviews.length} review(s):\n\n` +
      `${instructions}\n\n` +
      `For each review:\n` +
      `1. Read the file and follow the guidance for that review type\n` +
      `2. For question reviews, prefer answering in resolve_review.summary without code changes\n` +
      `3. Call resolve_review with the review_id and optional summary\n` +
      `   OR call dismiss_review with the review_id and a reason\n\n` +
      `The batch auto-resolves when all reviews are resolved or dismissed.`
    );
  }

  function reviewToolResult(review, headline) {
    const message = formatReviewSummary(review, headline);
    return {
      content: [{ type: "text", text: message }],
      structuredContent: {
        message,
        review,
      },
    };
  }

  // ── Router ────────────────────────────────────────────────────────

  const server = http.createServer(async (req, res) => {
    try {
      // GET /health
      if (req.method === "GET" && req.url === "/health") {
        sendJson(res, 200, { ok: true });
        return;
      }

      // GET /events — SSE stream for real-time comment status updates
      if (req.method === "GET" && req.url === "/events") {
        res.writeHead(200, {
          "content-type": "text/event-stream",
          "cache-control": "no-cache",
          connection: "keep-alive",
        });
        res.write(":\n\n"); // keep-alive comment
        sseClients.add(res);
        req.on("close", () => sseClients.delete(res));
        return;
      }

      // POST /reviews — create a review with individual comment rows
      if (req.method === "POST" && req.url === "/reviews") {
        const body = await readJson(req);
        const comments = reviewCommentSchema.array().min(1).parse(body.comments);
        const { reviewId, commentIds } = createReview(comments);
        sendJson(res, 201, {
          ok: true,
          id: reviewId,
          accepted_count: comments.length,
          comment_ids: commentIds,
        });
        return;
      }

      // GET /reviews/pending — claim and return the oldest pending review
      if (req.method === "GET" && req.url === "/reviews/pending") {
        const row = claimPending();
        if (!row) {
          sendJson(res, 200, { ok: true, review: null });
          return;
        }
        sendJson(res, 200, {
          ok: true,
          review: expandReview({ ...row, status: "in_progress" }),
        });
        return;
      }

      // GET /reviews/watch?timeout=120&batch_window=10 — long-poll for new reviews
      if (req.method === "GET" && req.url?.startsWith("/reviews/watch")) {
        const url = new URL(req.url, `http://${req.headers.host}`);
        const timeout = Math.min(Number(url.searchParams.get("timeout")) || 120, 300);
        const batchWindow = Math.min(Number(url.searchParams.get("batch_window")) || 10, 60);

        let closed = false;
        req.on("close", () => { closed = true; });

        const deadline = Date.now() + timeout * 1000;

        // Poll every 500ms until a pending review appears or timeout
        const found = await new Promise((resolve) => {
          const interval = setInterval(() => {
            if (closed) {
              clearInterval(interval);
              resolve(null);
              return;
            }
            const row = getPendingReview.get();
            if (row) {
              clearInterval(interval);
              resolve(row);
              return;
            }
            if (Date.now() >= deadline) {
              clearInterval(interval);
              resolve(null);
            }
          }, 500);
        });

        if (closed) return;

        if (!found) {
          sendJson(res, 200, { ok: true, review: null });
          return;
        }

        // Wait batch_window for more reviews to accumulate, then claim
        await new Promise((r) => setTimeout(r, batchWindow * 1000));
        if (closed) return;

        const claimed = claimPending();
        if (!claimed) {
          sendJson(res, 200, { ok: true, review: null });
          return;
        }
        sendJson(res, 200, {
          ok: true,
          review: expandReview({ ...claimed, status: "in_progress" }),
        });
        return;
      }

      // GET /reviews/:id — get a specific review with all comments
      const getReviewMatch = req.method === "GET" && req.url?.match(/^\/reviews\/([^/]+)$/);
      if (getReviewMatch) {
        const id = getReviewMatch[1];
        // Exclude /reviews/pending and /reviews/watch (already handled above)
        if (id !== "pending" && !id.startsWith("watch")) {
          const row = getReviewById.get(id);
          if (!row) {
            sendJson(res, 404, { ok: false, error: "review not found" });
            return;
          }
          sendJson(res, 200, { ok: true, review: expandReview(row) });
          return;
        }
      }

      // POST /reviews/:id/resolve — resolve entire review
      const resolveMatch = req.method === "POST" && req.url?.match(/^\/reviews\/([^/]+)\/resolve$/);
      if (resolveMatch) {
        const id = resolveMatch[1];
        const result = markReviewResolved.run(id);
        if (result.changes === 0) {
          sendJson(res, 404, { ok: false, error: "review not found" });
          return;
        }
        // Also resolve all open comments
        const openComments = getCommentsByReview.all(id).filter(
          (c) => c.status === "pending" || c.status === "acknowledged",
        );
        for (const c of openComments) {
          resolveComment.run("Resolved via review-level resolve", c.id);
          broadcastSSE("comment_status_changed", {
            review_id: id,
            comment_id: c.id,
            status: "resolved",
            summary: "Resolved via review-level resolve",
            dismiss_reason: null,
          });
        }
        sendJson(res, 200, { ok: true });
        return;
      }

      // POST /comments/:id/acknowledge
      const ackMatch = req.method === "POST" && req.url?.match(/^\/comments\/([^/]+)\/acknowledge$/);
      if (ackMatch) {
        const id = ackMatch[1];
        const comment = getCommentById.get(id);
        if (!comment) {
          sendJson(res, 404, { ok: false, error: "comment not found" });
          return;
        }
        const result = acknowledgeComment.run(id);
        if (result.changes === 0) {
          sendJson(res, 409, { ok: false, error: `cannot acknowledge comment in '${comment.status}' state` });
          return;
        }
        broadcastSSE("comment_status_changed", {
          review_id: comment.review_id,
          comment_id: id,
          status: "acknowledged",
          summary: null,
          dismiss_reason: null,
        });
        sendJson(res, 200, { ok: true });
        return;
      }

      // POST /comments/:id/resolve
      const resolveCommentMatch = req.method === "POST" && req.url?.match(/^\/comments\/([^/]+)\/resolve$/);
      if (resolveCommentMatch) {
        const id = resolveCommentMatch[1];
        const comment = getCommentById.get(id);
        if (!comment) {
          sendJson(res, 404, { ok: false, error: "comment not found" });
          return;
        }
        const body = await readJson(req).catch(() => ({}));
        const summary = body.summary ?? null;
        const result = resolveComment.run(summary, id);
        if (result.changes === 0) {
          sendJson(res, 409, { ok: false, error: `cannot resolve comment in '${comment.status}' state` });
          return;
        }
        broadcastSSE("comment_status_changed", {
          review_id: comment.review_id,
          comment_id: id,
          status: "resolved",
          summary,
          dismiss_reason: null,
        });
        autoResolveReview(comment.review_id);
        sendJson(res, 200, { ok: true });
        return;
      }

      // POST /comments/:id/dismiss
      const dismissMatch = req.method === "POST" && req.url?.match(/^\/comments\/([^/]+)\/dismiss$/);
      if (dismissMatch) {
        const id = dismissMatch[1];
        const comment = getCommentById.get(id);
        if (!comment) {
          sendJson(res, 404, { ok: false, error: "comment not found" });
          return;
        }
        const body = await readJson(req);
        const reason = body.reason;
        if (!reason || typeof reason !== "string") {
          sendJson(res, 400, { ok: false, error: "dismiss requires a reason" });
          return;
        }
        const result = dismissComment.run(reason, id);
        if (result.changes === 0) {
          sendJson(res, 409, { ok: false, error: `cannot dismiss comment in '${comment.status}' state` });
          return;
        }
        broadcastSSE("comment_status_changed", {
          review_id: comment.review_id,
          comment_id: id,
          status: "dismissed",
          summary: null,
          dismiss_reason: reason,
        });
        autoResolveReview(comment.review_id);
        sendJson(res, 200, { ok: true });
        return;
      }

      sendJson(res, 404, { ok: false, error: "not found" });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      sendJson(res, 400, { ok: false, error: message });
    }
  });

  await new Promise((resolve) => {
    server.listen(0, "127.0.0.1", resolve);
  });

  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error("server failed to bind a port");
  }

  await writeServerInfo({
    port: address.port,
    pid: process.pid,
    started_at: new Date().toISOString(),
  });

  const cleanup = async () => {
    // Close all SSE connections
    for (const client of sseClients) {
      try { client.end(); } catch {}
    }
    sseClients.clear();
    server.close();
    db.close();
    await removeServerInfoIfOwned(process.pid);
  };

  process.on("SIGINT", () => void cleanup().finally(() => process.exit(0)));
  process.on("SIGTERM", () => void cleanup().finally(() => process.exit(0)));
  process.on("exit", () => {
    try { db.close(); } catch {}
    try {
      const raw = readFileSync(STATE_PATH, "utf8");
      const info = JSON.parse(raw);
      if (info?.pid === process.pid) rmSync(STATE_PATH, { force: true });
    } catch {}
  });

  await new Promise(() => {});
}

// ── MCP mode (HTTP client to server) ────────────────────────────────

async function httpRequest(method, path, body, options = {}) {
  const info = await readServerInfo();
  if (!info) throw new Error("diff review server is not running");

  const url = `http://127.0.0.1:${info.port}${path}`;
  const reqOptions = { method, headers: {} };

  let payload;
  if (body !== undefined) {
    payload = JSON.stringify(body);
    reqOptions.headers["content-type"] = "application/json";
    reqOptions.headers["content-length"] = Buffer.byteLength(payload);
  }

  const timeout = options.timeout ?? 5000;

  return new Promise((resolve, reject) => {
    const req = http.request(url, reqOptions, (res) => {
      let data = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => (data += chunk));
      res.on("end", () => {
        try {
          resolve(JSON.parse(data));
        } catch {
          reject(new Error("invalid response from review server"));
        }
      });
    });

    req.on("error", (err) => reject(new Error(`review server unreachable: ${err.message}`)));
    req.setTimeout(timeout, () => {
      req.destroy();
      reject(new Error("review server request timed out"));
    });

    if (payload) req.write(payload);
    req.end();
  });
}

async function startMcpServer() {
  const server = new McpServer({
    name: "diff",
    version: "0.2.0",
  });

  // ── get_review ────────────────────────────────────────────────────

  server.registerTool(
    "get_review",
    {
      description:
        "Fetch a code review batch from diff and apply it. " +
        "Each review targets a specific file and line range. " +
        "You MUST read each referenced file and understand the review. " +
        "Action types: change-request (must fix in code), question (answer in resolve_review.summary without changing code unless explicitly required), nit (minor improvement or dismiss with reason). " +
        "After addressing each review, call resolve_review or dismiss_review. " +
        "When all reviews are handled, the batch auto-resolves.",
      inputSchema: {},
      outputSchema: reviewBatchOutputSchema,
    },
    async () => {
      try {
        const result = await httpRequest("GET", "/reviews/pending");
        if (!result.ok) {
          return { content: [{ type: "text", text: `Failed to fetch review: ${result.error}` }], isError: true };
        }

        if (!result.review) {
          return {
            content: [{ type: "text", text: "No reviews pending in diff." }],
            structuredContent: {
              message: "No reviews pending in diff.",
              review: null,
            },
          };
        }

        return reviewToolResult(result.review, "diff review batch");
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return { content: [{ type: "text", text: `Failed to read diff review: ${message}` }], isError: true };
      }
    },
  );

  // ── resolve_review ────────────────────────────────────────────────

  server.registerTool(
    "resolve_review",
    {
      description:
        "Mark a single review item inside a diff review batch as resolved. " +
        "Use `summary` to explain what changed, or to answer a question review without changing code.",
      inputSchema: {
        review_id: z.string().describe("The review item ID returned inside get_review or watch_reviews"),
        summary: z.string().optional().describe("Optional summary of what was done"),
      },
    },
    async ({ review_id, summary }) => {
      try {
        const body = summary ? { summary } : {};
        const result = await httpRequest("POST", `/comments/${review_id}/resolve`, body);
        if (!result.ok) {
          return { content: [{ type: "text", text: `Failed to resolve review: ${result.error}` }], isError: true };
        }
        return { content: [{ type: "text", text: `Review ${review_id} marked as resolved.` }] };
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return { content: [{ type: "text", text: `Failed to resolve review: ${message}` }], isError: true };
      }
    },
  );

  // ── watch_reviews ─────────────────────────────────────────────────

  server.registerTool(
    "watch_reviews",
    {
      description:
        "Block until a new code review batch arrives in diff, then return it. " +
        "Use in a loop for hands-free review processing. " +
        "After detecting the first new review, waits for a batch window before returning.",
      inputSchema: {
        timeoutSeconds: z.number().int().min(1).max(300).default(120)
          .describe("Max seconds to wait for a review batch (default 120)"),
        batchWindowSeconds: z.number().int().min(1).max(60).default(10)
          .describe("Seconds to wait after first review before returning (default 10)"),
      },
      outputSchema: reviewBatchOutputSchema,
    },
    async ({ timeoutSeconds, batchWindowSeconds }) => {
      try {
        const timeout = timeoutSeconds ?? 120;
        const batchWindow = batchWindowSeconds ?? 10;
        // Use a generous HTTP timeout: the full wait + buffer
        const httpTimeout = (timeout + batchWindow + 5) * 1000;
        const result = await httpRequest(
          "GET",
          `/reviews/watch?timeout=${timeout}&batch_window=${batchWindow}`,
          undefined,
          { timeout: httpTimeout },
        );
        if (!result.ok) {
          return { content: [{ type: "text", text: `Watch failed: ${result.error}` }], isError: true };
        }
        if (!result.review) {
          return {
            content: [{ type: "text", text: "No new reviews arrived within the timeout period." }],
            structuredContent: {
              message: "No new reviews arrived within the timeout period.",
              review: null,
            },
          };
        }

        return reviewToolResult(result.review, "New diff review batch");
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return { content: [{ type: "text", text: `Watch failed: ${message}` }], isError: true };
      }
    },
  );

  // ── dismiss_review ───────────────────────────────────────────────

  server.registerTool(
    "dismiss_review",
    {
      description:
        "Dismiss a single review item inside a diff review batch with an explanation. " +
        "Use when a review should not be addressed.",
      inputSchema: {
        review_id: z.string().describe("The review item ID to dismiss"),
        reason: z.string().min(1).describe("Reason for dismissing this review"),
      },
    },
    async ({ review_id, reason }) => {
      try {
        const result = await httpRequest("POST", `/comments/${review_id}/dismiss`, { reason });
        if (!result.ok) {
          return { content: [{ type: "text", text: `Failed to dismiss review: ${result.error}` }], isError: true };
        }
        return { content: [{ type: "text", text: `Review ${review_id} dismissed.` }] };
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return { content: [{ type: "text", text: `Failed to dismiss review: ${message}` }], isError: true };
      }
    },
  );

  const transport = new StdioServerTransport();
  await server.connect(transport);
}

// ── Entry point ─────────────────────────────────────────────────────

export async function main(argv = process.argv.slice(2)) {
  const [mode] = argv;

  if (mode === "server") {
    await startServer();
    return;
  }

  if (mode === "mcp") {
    await startMcpServer();
    return;
  }

  console.error("Usage: node sidecar/diff-mcp.js <server|mcp>");
  process.exit(1);
}

const entrypoint = process.argv[1] ? path.resolve(process.argv[1]) : null;
if (entrypoint === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`diff MCP sidecar failed: ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  });
}
