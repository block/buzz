import { createServer } from "node:http";

import { buildLessons, triageBatch } from "./classify.mjs";
import { describeMode } from "./llm.mjs";
import {
  fibresPayload,
  ingestedIds,
  listFeedback,
  listFibres,
  listIgnored,
  markIngested,
  patchFeedback,
  patchFibre,
  putFibres,
  recordFeedback,
  recordIgnored,
  recordTranscript,
  resetStore,
  restoreFibres,
} from "./store.mjs";

const PORT = Number(process.env.PORT ?? 8787);
const INGEST_BATCH = 15;

const CORS_HEADERS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, POST, PATCH, OPTIONS",
  "access-control-allow-headers": "content-type",
  "access-control-max-age": "86400",
};

function send(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    ...CORS_HEADERS,
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

async function readJson(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  if (chunks.length === 0) return {};
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

function chunk(items, size) {
  const batches = [];
  for (let index = 0; index < items.length; index += size) {
    batches.push(items.slice(index, index + size));
  }
  return batches;
}

async function ingestMessages(pubkey, incoming) {
  const known = ingestedIds(pubkey);
  const unseen = incoming
    .filter((message) => message?.eventId && !known.has(message.eventId))
    .filter((message) => (message.content ?? "").trim().length > 0)
    .sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));

  // Context comes from the transcript, so it has to hold every message —
  // including the ones triage goes on to ignore.
  recordTranscript(pubkey, unseen);

  const allChanges = [];
  let ignoredCount = 0;

  for (const batch of chunk(unseen, INGEST_BATCH)) {
    const result = await triageBatch({
      pubkey,
      messages: batch,
      fibres: listFibres(pubkey),
      lessons: buildLessons(listFeedback(pubkey)),
    });
    putFibres(pubkey, result.fibres);
    markIngested(pubkey, result.ingestedEventIds);
    recordIgnored(pubkey, result.ignored);
    allChanges.push(...result.changes);
    ignoredCount += result.ignored.length;
  }

  if (unseen.length === 0 && incoming.length > 0) {
    markIngested(
      pubkey,
      incoming.map((message) => message.eventId).filter(Boolean),
    );
  }

  const payload = fibresPayload(pubkey);
  return {
    ...payload,
    changes: allChanges,
    ingested: unseen.length,
    ignored: ignoredCount,
  };
}

async function route(req, url) {
  const { pathname, searchParams } = url;
  const method = req.method ?? "GET";

  if (method === "GET" && pathname === "/health") {
    return [200, { status: "ok", mode: describeMode() }];
  }

  if (method === "POST" && pathname === "/ingest") {
    const body = await readJson(req);
    const pubkey = body.pubkey;
    const messages = Array.isArray(body.messages) ? body.messages : [];
    if (!pubkey) return [400, { error: "pubkey is required" }];
    const result = await ingestMessages(pubkey, messages);
    console.log(
      `[triage] ${pubkey.slice(0, 8)}: ${result.ingested} messages in, ${result.ignored} ignored, ${result.openCount} open fibres (${result.laneCounts.important} important, ${result.laneCounts.hot} hot, ${result.laneCounts.other} other)`,
    );
    return [200, result];
  }

  if (method === "GET" && pathname === "/fibres") {
    const pubkey = searchParams.get("pubkey");
    if (!pubkey) return [400, { error: "pubkey is required" }];
    return [200, fibresPayload(pubkey)];
  }

  if (method === "GET" && pathname === "/ignored") {
    const pubkey = searchParams.get("pubkey");
    if (!pubkey) return [400, { error: "pubkey is required" }];
    const ignored = listIgnored(pubkey);
    return [
      200,
      {
        ignored,
        count: ignored.length,
        addressedCount: ignored.filter((entry) => entry.wasAddressed).length,
      },
    ];
  }

  const fibreMatch = pathname.match(/^\/fibres\/([\w-]+)$/);
  if (method === "PATCH" && fibreMatch) {
    const body = await readJson(req);
    if (!body.pubkey) return [400, { error: "pubkey is required" }];
    if (!["done", "dismissed", "open"].includes(body.status)) {
      return [400, { error: "status must be done, dismissed, or open" }];
    }
    const fibre = patchFibre(body.pubkey, fibreMatch[1], {
      status: body.status,
    });
    return fibre
      ? [200, { fibre, ...fibresPayload(body.pubkey) }]
      : [404, { error: "fibre not found" }];
  }

  if (method === "POST" && pathname === "/fibres/restore") {
    const body = await readJson(req);
    if (!body.pubkey) return [400, { error: "pubkey is required" }];
    restoreFibres(body.pubkey);
    return [200, fibresPayload(body.pubkey)];
  }

  if (method === "POST" && pathname === "/feedback") {
    const body = await readJson(req);
    if (!body.pubkey || !body.fibreId) {
      return [400, { error: "pubkey and fibreId are required" }];
    }
    const feedback = recordFeedback(body.pubkey, body);
    return [201, { feedback }];
  }

  const feedbackMatch = pathname.match(/^\/feedback\/([\w-]+)$/);
  if (method === "PATCH" && feedbackMatch) {
    const body = await readJson(req);
    if (!body.pubkey) return [400, { error: "pubkey is required" }];
    const note = typeof body.note === "string" ? body.note.trim() : "";
    if (!note) return [400, { error: "note is required" }];
    const feedback = patchFeedback(body.pubkey, feedbackMatch[1], { note });
    return feedback
      ? [200, { feedback }]
      : [404, { error: "feedback not found" }];
  }

  if (method === "POST" && pathname === "/reset") {
    resetStore();
    console.log("[triage] store purged");
    return [200, { status: "ok" }];
  }

  return [404, { error: `no route for ${method} ${pathname}` }];
}

createServer(async (req, res) => {
  if (req.method === "OPTIONS") {
    res.writeHead(204, CORS_HEADERS);
    res.end();
    return;
  }

  const url = new URL(req.url ?? "/", `http://localhost:${PORT}`);

  try {
    const [status, body] = await route(req, url);
    send(res, status, body);
  } catch (error) {
    console.error(`[triage] ${req.method} ${url.pathname} failed`, error);
    send(res, 500, { error: error.message });
  }
}).listen(PORT, () => {
  console.log(
    `[triage] listening on http://localhost:${PORT} (${describeMode()} mode)`,
  );
});
