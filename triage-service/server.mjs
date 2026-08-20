import { createServer } from "node:http";

import { classify } from "./classify.mjs";
import {
  createTodo,
  getSuggestions,
  listFeedback,
  listTodos,
  patchSuggestion,
  putSuggestions,
  recordFeedback,
  updateTodo,
} from "./store.mjs";

const PORT = Number(process.env.PORT ?? 8787);

// The Tauri webview origin varies (tauri://localhost, http://localhost:1420),
// so this PoC accepts any origin rather than maintaining an allowlist.
const CORS_HEADERS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, POST, PATCH, OPTIONS",
  "access-control-allow-headers": "content-type",
  "access-control-max-age": "86400",
};

/**
 * How a user decision rewrites the stored verdict. The client renders its
 * actions from `verdict`, so promotion has to actually change it — otherwise
 * an item moves lists while still offering the old buttons. `adopted` keeps a
 * message out of Important once it owns a todo.
 */
function suggestionPatchFor(userAction) {
  switch (userAction) {
    case "promoted":
      return {
        verdict: "attention",
        learned: true,
        reason: "You told me this message matters.",
        confidence: 1,
      };
    case "dismissed":
      return {
        verdict: "noise",
        learned: true,
        reason: "You dismissed this message.",
        confidence: 1,
        adopted: false,
      };
    case "adopted":
      return { adopted: true };
    case "completed":
      return { adopted: true };
    default:
      return null;
  }
}

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

async function route(req, url) {
  const { pathname, searchParams } = url;
  const method = req.method ?? "GET";

  if (method === "GET" && pathname === "/health") {
    return [200, { status: "ok" }];
  }

  if (method === "POST" && pathname === "/scan") {
    const body = await readJson(req);
    const pubkey = body.pubkey;
    const candidates = Array.isArray(body.candidates) ? body.candidates : [];
    if (!pubkey) return [400, { error: "pubkey is required" }];

    const verdicts = await classify(candidates, listFeedback(pubkey));

    // Store a renderable snapshot alongside each verdict so the client can
    // reload the triage view without re-collecting candidates from the relay.
    const byEventId = new Map(
      candidates.map((candidate) => [candidate.eventId, candidate]),
    );
    // A rescan replaces the whole result set, so decisions already made have to
    // be carried forward or an adopted message reappears in Important.
    const adoptedEventIds = new Set(
      getSuggestions(pubkey)
        .filter((previous) => previous.adopted)
        .map((previous) => previous.eventId),
    );
    const suggestions = verdicts.map((verdict) => {
      const candidate = byEventId.get(verdict.eventId);
      return {
        ...verdict,
        adopted: adoptedEventIds.has(verdict.eventId),
        channelName: candidate?.channelName ?? null,
        authorPubkey: candidate?.authorPubkey ?? null,
        authorLabel: candidate?.authorLabel ?? null,
        content: (candidate?.content ?? "").slice(0, 2000),
        createdAt: candidate?.createdAt ?? null,
        isDm: candidate?.isDm ?? false,
        isMention: candidate?.isMention ?? false,
      };
    });

    putSuggestions(pubkey, suggestions);
    console.log(
      `[triage] scanned ${candidates.length} candidates for ${pubkey.slice(0, 8)}: ` +
        `${suggestions.filter((s) => s.verdict === "attention").length} attention, ` +
        `${suggestions.filter((s) => s.verdict === "noise").length} noise`,
    );
    return [200, { suggestions }];
  }

  if (method === "GET" && pathname === "/suggestions") {
    const pubkey = searchParams.get("pubkey");
    if (!pubkey) return [400, { error: "pubkey is required" }];
    return [200, { suggestions: getSuggestions(pubkey) }];
  }

  if (method === "GET" && pathname === "/todos") {
    const pubkey = searchParams.get("pubkey");
    if (!pubkey) return [400, { error: "pubkey is required" }];
    return [200, { todos: listTodos(pubkey) }];
  }

  if (method === "POST" && pathname === "/todos") {
    const body = await readJson(req);
    if (!body.pubkey || !body.eventId) {
      return [400, { error: "pubkey and eventId are required" }];
    }
    return [201, { todo: createTodo(body.pubkey, body) }];
  }

  const todoMatch = pathname.match(/^\/todos\/([\w-]+)$/);
  if (method === "PATCH" && todoMatch) {
    const body = await readJson(req);
    if (!body.pubkey) return [400, { error: "pubkey is required" }];
    if (!["done", "dismissed", "open"].includes(body.status)) {
      return [400, { error: "status must be done, dismissed, or open" }];
    }
    const todo = updateTodo(body.pubkey, todoMatch[1], body.status);
    return todo ? [200, { todo }] : [404, { error: "todo not found" }];
  }

  if (method === "POST" && pathname === "/feedback") {
    const body = await readJson(req);
    if (!body.pubkey || !body.eventId) {
      return [400, { error: "pubkey and eventId are required" }];
    }

    const feedback = recordFeedback(body.pubkey, body);
    const patch = suggestionPatchFor(body.userAction);
    if (patch) {
      patchSuggestion(body.pubkey, body.eventId, patch);
    }
    return [201, { feedback }];
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
  const mode = process.env.TRIAGE_LLM === "1" ? "LLM" : "heuristic";
  console.log(`[triage] listening on http://localhost:${PORT} (${mode} mode)`);
});
