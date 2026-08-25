import assert from "node:assert/strict";
import test from "node:test";

import {
  parseInboundBusinessBridgeMessage,
  readBusinessBridgeEvent,
} from "./businessDockBridge.ts";

const config = {
  homeUrl: "https://biz.example.com/embed/",
  origin: "https://biz.example.com",
};
const source = {};
const nonce = "session-nonce";

function event(overrides = {}) {
  return {
    data: { version: 1, type: "BUSINESS_READY" },
    origin: config.origin,
    source,
    ...overrides,
  };
}

test("business bridge accepts a valid message from the configured frame", () => {
  assert.deepEqual(readBusinessBridgeEvent(event(), source, config), {
    version: 1,
    type: "BUSINESS_READY",
    requestId: undefined,
  });
});

test("business bridge ignores the wrong origin and source", () => {
  assert.equal(
    readBusinessBridgeEvent(
      event({ origin: "https://evil.example" }),
      source,
      config,
    ),
    null,
  );
  assert.equal(readBusinessBridgeEvent(event(), {}, config), null);
});

test("business bridge ignores invalid versions, message shapes, and types", () => {
  for (const data of [
    null,
    { version: 2, type: "BUSINESS_READY" },
    { version: 1, type: "UNKNOWN" },
    { version: 1, type: "TITLE_CHANGED", payload: { title: 3 } },
  ]) {
    assert.equal(parseInboundBusinessBridgeMessage(data), null);
  }
});

test("business bridge rejects a route outside the configured origin", () => {
  assert.equal(
    readBusinessBridgeEvent(
      event({
        data: {
          version: 1,
          type: "ROUTE_CHANGED",
          payload: { url: "https://evil.example/orders/42" },
        },
      }),
      source,
      config,
    ),
    null,
  );
});

function v2(type, payload, overrides = {}) {
  return event({
    data: {
      version: 2,
      type,
      requestId: "req-1",
      sessionNonce: nonce,
      ...(payload === undefined ? {} : { payload }),
      ...overrides,
    },
  });
}

function v3(type, payload, overrides = {}) {
  return event({
    data: {
      version: 3,
      type,
      requestId: "req-auth-1",
      sessionNonce: nonce,
      payload,
      ...overrides,
    },
  });
}

test("business bridge V3 accepts only minimal auth status", () => {
  assert.deepEqual(
    readBusinessBridgeEvent(
      v3("AUTH_STATUS", {
        authenticated: true,
        user: { subject: "user-1", displayName: "Ada" },
      }),
      source,
      config,
      nonce,
    )?.payload,
    {
      authenticated: true,
      user: { subject: "user-1", displayName: "Ada" },
    },
  );
  for (const leaked of [
    { token: "secret" },
    { accessToken: "secret" },
    { groups: ["admins"] },
    { email: "ada@example.com" },
  ]) {
    assert.equal(
      readBusinessBridgeEvent(
        v3("AUTH_STATUS", {
          authenticated: true,
          user: { subject: "user-1", displayName: "Ada" },
          ...leaked,
        }),
        source,
        config,
        nonce,
      ),
      null,
    );
  }
  assert.equal(
    readBusinessBridgeEvent(
      v3("AUTH_STATUS", {
        authenticated: true,
        user: {
          subject: "user-1",
          displayName: "Ada",
          token: "secret",
        },
      }),
      source,
      config,
      nonce,
    ),
    null,
  );
});

test("business bridge V3 validates required and expired events", () => {
  assert.equal(
    readBusinessBridgeEvent(
      v3("AUTH_REQUIRED", { reason: "No Business cookie" }),
      source,
      config,
      nonce,
    )?.type,
    "AUTH_REQUIRED",
  );
  assert.equal(
    readBusinessBridgeEvent(
      v3("SESSION_EXPIRED", { reason: "Expired", token: "no" }),
      source,
      config,
      nonce,
    ),
    null,
  );
  assert.equal(
    readBusinessBridgeEvent(
      v3("AUTH_REQUIRED", {}, { sessionNonce: "wrong" }),
      source,
      config,
      nonce,
    ),
    null,
  );
});

test("business bridge accepts V2 only with the matching nonce", () => {
  assert.equal(
    readBusinessBridgeEvent(v2("BUSINESS_READY"), source, config, nonce)
      ?.version,
    2,
  );
  assert.equal(
    readBusinessBridgeEvent(
      v2("BUSINESS_READY", undefined, { sessionNonce: "wrong" }),
      source,
      config,
      nonce,
    ),
    null,
  );
  assert.equal(
    readBusinessBridgeEvent(v2("BUSINESS_READY"), {}, config, nonce),
    null,
  );
});

test("business bridge validates V2 resource and dirty payloads", () => {
  const resource = {
    version: 1,
    type: "sales_order",
    id: "SO-1",
    path: "/embed/sales-orders/SO-1",
  };
  assert.deepEqual(
    readBusinessBridgeEvent(
      v2("RESOURCE_CHANGED", { resource }),
      source,
      config,
      nonce,
    )?.payload,
    { resource },
  );
  assert.deepEqual(
    readBusinessBridgeEvent(
      v2("DIRTY_STATE_CHANGED", { dirty: true }),
      source,
      config,
      nonce,
    )?.payload,
    { dirty: true },
  );
  assert.equal(
    readBusinessBridgeEvent(
      v2("DIRTY_STATE_CHANGED", { dirty: "yes" }),
      source,
      config,
      nonce,
    ),
    null,
  );
});

test("business bridge accepts safe action events and rejects invalid payloads", () => {
  for (const type of ["ACTION_COMPLETED", "ACTION_FAILED"]) {
    const message = readBusinessBridgeEvent(
      v2(type, {
        action: "work_item_created",
        message:
          type === "ACTION_COMPLETED" ? "Work item created" : "Creation failed",
        resource: { type: "work_item", id: "WI-1" },
        traceId: "trace-1",
        stack: "must not escape parsing",
      }),
      source,
      config,
      nonce,
    );
    assert.equal(message?.type, type);
    assert.equal("stack" in message.payload, false);
  }
  assert.equal(
    readBusinessBridgeEvent(
      v2("ACTION_FAILED", { message: "missing action" }),
      source,
      config,
      nonce,
    ),
    null,
  );
  assert.equal(
    readBusinessBridgeEvent(
      v2("ACTION_COMPLETED", {
        action: "approve_order",
        message: "Authority-changing action must be rejected",
      }),
      source,
      config,
      nonce,
    ),
    null,
  );
});

test("business bridge validates data change events", () => {
  const resource = {
    version: 1,
    type: "invoice",
    id: "INV-1",
    path: "/embed/invoices/INV-1",
  };
  const message = readBusinessBridgeEvent(
    v2("DATA_CHANGED", { resource, traceId: "trace-data" }),
    source,
    config,
    nonce,
  );
  assert.equal(message?.type, "DATA_CHANGED");
  assert.equal(message?.payload.resource.id, "INV-1");
  assert.equal(
    readBusinessBridgeEvent(
      v2("DATA_CHANGED", { resource: { ...resource, path: "/../admin" } }),
      source,
      config,
      nonce,
    ),
    null,
  );
});
