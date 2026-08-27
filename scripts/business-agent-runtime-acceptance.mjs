#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const expectedTools = [
  "analyze_cross_domain_risks",
  "analyze_inventory_risks",
  "analyze_order_profit_risks",
  "analyze_purchase_cost_risks",
  "analyze_receivable_risks",
  "create_customer_receipt_draft",
  "create_goods_receipt_draft",
  "create_purchase_order_draft",
  "create_sales_order_draft",
  "create_shipment_draft",
  "create_supplier_payment_draft",
  "explain_profit_change",
  "get_action_proposal",
  "get_action_recommendations",
  "get_approval_draft",
  "get_business_anomaly",
  "get_business_data_quality",
  "get_finding_lifecycle",
  "get_management_profit_report",
  "get_management_report_snapshot",
  "get_operating_dashboard",
  "get_profit_evidence",
  "get_purchase_order",
  "get_sales_order",
  "get_work_item",
  "query_inventory_balance",
  "query_order_profit",
  "query_payables",
  "query_profitability_by_dimension",
  "query_receivables",
  "search_business_anomalies",
  "search_purchase_orders",
  "search_sales_orders",
  "search_work_items",
].map((name) => `business-read-mcp__${name}`);

function jsonResponse(response, body) {
  response.writeHead(200, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

const observedRequests = [];
const modelServer = http.createServer((request, response) => {
  let raw = "";
  request.setEncoding("utf8");
  request.on("data", (chunk) => {
    raw += chunk;
  });
  request.on("end", () => {
    const body = JSON.parse(raw);
    observedRequests.push(body);
    jsonResponse(response, {
      id: "business-agent-runtime-acceptance",
      object: "chat.completion",
      created: 0,
      model: "business-agent-probe",
      choices: [
        {
          index: 0,
          message: { role: "assistant", content: "probe complete" },
          finish_reason: "stop",
        },
      ],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    });
  });
});

await new Promise((resolve, reject) => {
  modelServer.once("error", reject);
  modelServer.listen(0, "127.0.0.1", resolve);
});

const address = modelServer.address();
assert(address && typeof address !== "string");
const agent = spawn(path.join(repoRoot, "target/debug/buzz-agent"), [], {
  cwd: repoRoot,
  env: {
    ...process.env,
    BUZZ_AGENT_PROVIDER: "openai",
    OPENAI_COMPAT_API_KEY: "acceptance-probe-not-used",
    OPENAI_COMPAT_MODEL: "business-agent-probe",
    OPENAI_COMPAT_API: "chat",
    OPENAI_COMPAT_BASE_URL: `http://127.0.0.1:${address.port}/v1`,
    BUZZ_AGENT_NO_HINTS: "1",
  },
  stdio: ["pipe", "pipe", "inherit"],
});

let nextRequestId = 0;
const pending = new Map();
const lines = readline.createInterface({ input: agent.stdout });
lines.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id === undefined || message.id === null) return;
  const waiter = pending.get(message.id);
  if (!waiter) return;
  pending.delete(message.id);
  if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
  else waiter.resolve(message.result);
});

function request(method, params) {
  const id = nextRequestId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    agent.stdin.write(
      `${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`,
    );
  });
}

const timeout = setTimeout(() => {
  agent.kill("SIGKILL");
}, 20_000);

try {
  const initialized = await request("initialize", {
    protocolVersion: 2,
    clientCapabilities: {},
    clientInfo: { name: "business-agent-runtime-acceptance", version: "1" },
  });
  assert.equal(
    initialized.agentInfo?.name ?? initialized.serverInfo?.name,
    "buzz-agent",
  );

  const session = await request("session/new", {
    cwd: repoRoot,
    systemPrompt: "Business Agent runtime acceptance probe.",
    mcpServers: [
      {
        name: "business-read-mcp",
        command: path.join(repoRoot, "target/debug/business-read-mcp"),
        args: [],
        env: [
          { name: "BUSINESS_READ_ADAPTER", value: "mock" },
          {
            name: "BUSINESS_READ_MOCK_ACKNOWLEDGE",
            value: "Mock Only - Production Disabled",
          },
          {
            name: "BUSINESS_READ_SERVICE_CREDENTIAL",
            value: "0123456789abcdef0123456789abcdef",
          },
          {
            name: "BUSINESS_AGENT_DELEGATION_TOKEN",
            value: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
          },
          { name: "BUSINESS_AGENT_ID", value: "acceptance-agent" },
          { name: "BUSINESS_AGENT_TURN_ID", value: "acceptance-turn" },
          {
            name: "BUSINESS_AGENT_TRACE_ID",
            value: "00000000-0000-4000-8000-000000000001",
          },
          {
            name: "BUSINESS_AUTH_GATEWAY_BASE_URL",
            value: "http://127.0.0.1:9/",
          },
          { name: "BUSINESS_READ_SERVICE_AUTH_MODE", value: "shared_secret" },
          {
            name: "BUSINESS_READ_SERVICE_AUDIENCE",
            value: "business-read-api",
          },
          { name: "BUSINESS_ANOMALY_ENABLED", value: "true" },
          { name: "BUSINESS_AGENT_DRAFT_WRITE_ENABLED", value: "true" },
        ],
      },
    ],
  });

  const prompt = await request("session/prompt", {
    sessionId: session.sessionId,
    prompt: [
      {
        type: "text",
        text: "Reply with probe complete without calling a tool.",
      },
    ],
  });
  assert.equal(prompt.stopReason, "end_turn");
  assert.equal(observedRequests.length, 1);

  const modelTools = observedRequests[0].tools
    .map((tool) => tool.function?.name ?? tool.name)
    .filter(Boolean)
    .sort();
  assert.deepEqual(modelTools, expectedTools.sort());

  console.log(
    JSON.stringify({
      runtime: "buzz-agent",
      mcp: "business-read-mcp",
      modelVisibleTools: modelTools.length,
      onlyFixedBusinessTools: true,
      promptCompleted: true,
    }),
  );
} finally {
  clearTimeout(timeout);
  lines.close();
  agent.kill("SIGTERM");
  await new Promise((resolve) => modelServer.close(resolve));
}
