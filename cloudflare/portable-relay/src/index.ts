import { httpStatusForDenial, type DenialCode } from "./identity";
import {
  RelayNode,
  STABLE_NODE_KEY_HEADER,
  type HttpAuthEvidence,
  type RpcOutcome,
} from "./relay-node";
import {
  eventFromUnknown,
  filtersFromUnknown,
  ProtocolInputError,
} from "./protocol";
import { replicationRecordsFromUnknown } from "./replication";
import { StableNodeKeyError, stableNodeKeyFromUrl } from "./stable-node-key";

export { RelayNode };

const PORTABLE_HTTP_PATHS = new Set([
  "/events",
  "/query",
  "/count",
  "/replication",
]);
const MAX_HTTP_BODY_BYTES = 256 * 1024;

export default {
  async fetch(request, env): Promise<Response> {
    const url = new URL(request.url);

    if (request.method === "GET" && url.pathname === "/health") {
      return json({
        status: "ok",
        adapter: "portable-relay-cloudflare-v0.1",
        implementation: "portable-core-candidate",
      });
    }

    const isWebSocketRoute = url.pathname === "/";
    if (!isWebSocketRoute && !PORTABLE_HTTP_PATHS.has(url.pathname)) {
      return json({ error: "not_found" }, 404);
    }
    if (
      isWebSocketRoute &&
      (request.method !== "GET" ||
        request.headers.get("Upgrade")?.toLowerCase() !== "websocket")
    ) {
      return json({ error: "websocket_upgrade_required" }, 426);
    }
    if (!isWebSocketRoute && request.method !== "POST") {
      return json({ error: "method_not_allowed" }, 405, { Allow: "POST" });
    }

    let stableNodeKey: string;
    try {
      stableNodeKey = stableNodeKeyFromUrl(url);
    } catch (error) {
      if (!(error instanceof StableNodeKeyError)) {
        throw error;
      }
      return json(
        { error: "invalid_stable_node", message: error.message },
        400,
      );
    }

    const node = env.RELAY_NODES.getByName(stableNodeKey);
    if (isWebSocketRoute) {
      const headers = new Headers(request.headers);
      headers.set(STABLE_NODE_KEY_HEADER, stableNodeKey);
      return node.fetch(new Request(request, { headers }));
    }

    let body: unknown;
    let bytes: ArrayBuffer;
    try {
      bytes = await readBody(request);
      body = parseJson(bytes);
    } catch (error) {
      if (!(error instanceof ProtocolInputError)) {
        throw error;
      }
      return json({ error: "invalid_request", message: error.message }, 400);
    }
    const auth: HttpAuthEvidence = {
      authorization: request.headers.get("authorization"),
      url: request.url,
      method: request.method,
      payloadSha256Hex: await sha256Hex(bytes),
    };

    try {
      if (url.pathname === "/events") {
        return unwrap(
          await node.submitEvent(stableNodeKey, eventFromUnknown(body), auth),
        );
      }
      if (url.pathname === "/replication") {
        return unwrap(
          await node.ingestReplication(
            stableNodeKey,
            replicationRecordsFromUnknown(body),
            auth,
          ),
        );
      }

      const filters = filtersFromUnknown(body);
      if (url.pathname === "/query") {
        return unwrap(await node.queryEvents(stableNodeKey, filters, auth));
      }
      const counted = await node.countEvents(stableNodeKey, filters, auth);
      return "denied" in counted
        ? denialResponse(counted.denied)
        : json({ count: counted.result });
    } catch (error) {
      if (error instanceof ProtocolInputError) {
        return json({ error: "invalid_request", message: error.message }, 400);
      }
      console.error("portable relay operation failed", {
        stableNodeKey,
        path: url.pathname,
        error:
          error instanceof Error ? error.name : "unknown_operational_error",
      });
      return json({ error: "relay_operation_failed" }, 500);
    }
  },
} satisfies ExportedHandler<Env>;

async function readBody(request: Request): Promise<ArrayBuffer> {
  const declaredLength = request.headers.get("Content-Length");
  if (
    declaredLength !== null &&
    Number.parseInt(declaredLength, 10) > MAX_HTTP_BODY_BYTES
  ) {
    throw new ProtocolInputError("request body exceeds 256 KiB");
  }

  const bytes = await request.arrayBuffer();
  if (bytes.byteLength > MAX_HTTP_BODY_BYTES) {
    throw new ProtocolInputError("request body exceeds 256 KiB");
  }
  return bytes;
}

function parseJson(bytes: ArrayBuffer): unknown {
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    throw new ProtocolInputError("request body is not valid JSON");
  }
}

async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

function unwrap<T>(outcome: RpcOutcome<T>): Response {
  return "denied" in outcome
    ? denialResponse(outcome.denied)
    : json(outcome.result);
}

function denialResponse(code: DenialCode): Response {
  return json({ error: "identity_denied", code }, httpStatusForDenial(code));
}

function json(
  body: unknown,
  status = 200,
  extraHeaders: HeadersInit = {},
): Response {
  return Response.json(body, {
    status,
    headers: {
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
      ...extraHeaders,
    },
  });
}
