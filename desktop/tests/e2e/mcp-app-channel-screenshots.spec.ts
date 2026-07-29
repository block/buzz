import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const OUTDIR = "test-results/pr-3275-neutral";
const ENDPOINT = "https://runtime.example.test/mcp";
const RESOURCE_URI = "ui://review/project-board";
const SANDBOX_URL = "http://buzz-mcp-app.localhost/project-board";

const POLICY = {
  csp: {
    connectDomains: ["https://api.example.test"],
    resourceDomains: ["https://assets.example.test"],
    frameDomains: [],
    baseUriDomains: [],
  },
  requestedPermissions: { clipboardWrite: {} },
};

const SERVER = {
  serverId: "review-project-board",
  endpoint: ENDPOINT,
  name: "Review workspace",
  version: "1.0.0",
  protocolVersion: "2025-11-25",
  tools: [
    {
      name: "project_board",
      title: "Project board",
      description: "Review project work by status.",
      inputSchema: { type: "object", properties: {} },
      outputSchema: null,
      annotations: null,
      meta: {},
      uiResourceUri: RESOURCE_URI,
      visibility: ["app", "model"],
    },
  ],
  resources: [
    {
      uri: RESOURCE_URI,
      name: "Project board",
      title: "Project board",
      description: "A neutral review fixture.",
      mimeType: "text/html;profile=mcp-app",
      meta: {},
    },
  ],
};

const APP_HTML = String.raw`<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <style>
    :root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;background:#1d1d24;color:#eee9d2}
    *{box-sizing:border-box}body{margin:0;padding:22px;background:#1d1d24}
    header{display:flex;align-items:center;justify-content:space-between;margin-bottom:18px}
    h1{font-size:16px;margin:0}.eyebrow{color:#a4a0ae;font-size:12px;margin-top:4px}
    .live{border:1px solid #3a3b43;border-radius:999px;color:#c9c5b2;font-size:12px;padding:6px 10px}
    .live::before{background:#34c77b;border-radius:50%;content:"";display:inline-block;height:7px;margin-right:6px;width:7px}
    .board{display:grid;grid-template-columns:repeat(4,minmax(175px,1fr));gap:12px}
    .column{border:1px solid #383840;border-radius:14px;background:#24242c;min-height:430px;padding:12px}
    .column-title{display:flex;justify-content:space-between;font-size:13px;font-weight:650;margin-bottom:12px}
    .count{background:#1b1b21;border-radius:999px;color:#aaa6b2;font-size:11px;padding:2px 7px}
    .card{background:#1f1f27;border:1px solid #3c3c46;border-radius:10px;margin-bottom:10px;padding:11px}
    .card strong{display:block;font-size:13px;line-height:1.35}.meta{color:#9995a3;font-size:11px;margin-top:8px}
    .priority{color:#7fa8ff}.done{color:#5bc68a}
    button{background:#3478f6;border:0;border-radius:9px;color:white;cursor:pointer;font:inherit;font-size:12px;padding:8px 12px}
    button:disabled{cursor:default;opacity:.65}
  </style>
</head>
<body>
  <header>
    <div><h1>Project board</h1><div class="eyebrow">Shared channel view · MCP App</div></div>
    <div><span class="live">Connected</span> <button id="share">Share update</button></div>
  </header>
  <main class="board">
    <section class="column"><div class="column-title">Backlog <span class="count">2</span></div>
      <article class="card"><strong>Document release checklist</strong><div class="meta">Docs · Normal</div></article>
      <article class="card"><strong>Define empty-state copy</strong><div class="meta">Product · Normal</div></article>
    </section>
    <section class="column"><div class="column-title">Active <span class="count">2</span></div>
      <article class="card"><strong>Review API contract</strong><div class="meta"><span class="priority">High</span> · Platform</div></article>
      <article class="card"><strong>Run accessibility pass</strong><div class="meta">Desktop · Normal</div></article>
    </section>
    <section class="column"><div class="column-title">Review <span class="count">1</span></div>
      <article class="card"><strong>Validate channel app lifecycle</strong><div class="meta"><span class="priority">High</span> · Desktop</div></article>
    </section>
    <section class="column"><div class="column-title">Done <span class="count">2</span></div>
      <article class="card"><strong>Threat-model sandbox boundary</strong><div class="meta"><span class="done">Complete</span> · Security</div></article>
      <article class="card"><strong>Add protocol compatibility probe</strong><div class="meta"><span class="done">Complete</span> · Runtime</div></article>
    </section>
  </main>
  <script>
    const pending = new Map();
    let requestId = 0;
    function send(message){ parent.postMessage(message, "*"); }
    function request(method, params){
      const id = ++requestId;
      send({jsonrpc:"2.0",id,method,params});
      return new Promise((resolve,reject)=>pending.set(id,{resolve,reject}));
    }
    window.addEventListener("message",(event)=>{
      if(event.source!==parent)return;
      const message=event.data;
      if(message && pending.has(message.id) && ("result" in message || "error" in message)){
        const waiter=pending.get(message.id);pending.delete(message.id);
        if(message.error)waiter.reject(new Error(message.error.message));else waiter.resolve(message.result);
        return;
      }
      if(message?.method==="ui/resource-teardown" && message.id!==undefined){
        send({jsonrpc:"2.0",id:message.id,result:{}});
      }
    });
    document.getElementById("share").addEventListener("click",async(event)=>{
      event.currentTarget.disabled=true;
      try{
        await request("ui/message",{role:"user",content:[{type:"text",text:"Board update:\n\nMove “Review API contract” to Review."}]});
      }finally{event.currentTarget.disabled=false}
    });
    (async()=>{
      await request("ui/initialize",{
        appInfo:{name:"Project board",version:"1.0.0"},
        appCapabilities:{availableDisplayModes:["inline"]},
        protocolVersion:"2026-01-26"
      });
      send({jsonrpc:"2.0",method:"ui/notifications/initialized",params:{}});
    })();
  </script>
</body>
</html>`;

async function installMcpAppCommandMocks(
  page: import("@playwright/test").Page,
) {
  await page.evaluate(
    ({ appHtml, policy, sandboxUrl, server }) => {
      const testWindow = window as Window & {
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };
      const originalInvoke = testWindow.__TAURI_INTERNALS__?.invoke?.bind(
        testWindow.__TAURI_INTERNALS__,
      );
      if (!originalInvoke || !testWindow.__TAURI_INTERNALS__) {
        throw new Error("Mock Tauri invoke bridge is unavailable.");
      }
      testWindow.__TAURI_INTERNALS__.invoke = async (command, payload) => {
        switch (command) {
          case "connect_mcp_app_server":
            return server;
          case "inspect_mcp_app_resource":
            return policy;
          case "prepare_mcp_app_view":
            return {
              viewId: "project-board-view",
              sandboxUrl,
              html: appHtml,
              csp: policy.csp,
              requestedPermissions: policy.requestedPermissions,
            };
          case "call_mcp_app_tool":
            return {
              content: [{ type: "text", text: "Project board ready." }],
            };
          case "list_mcp_app_resources":
            return server.resources;
          case "read_mcp_app_resource":
            return { contents: [] };
          case "disconnect_mcp_app_server":
          case "release_mcp_app_view":
            return undefined;
          default:
            return originalInvoke(command, payload);
        }
      };
    },
    {
      appHtml: APP_HTML,
      policy: POLICY,
      sandboxUrl: SANDBOX_URL,
      server: SERVER,
    },
  );
}

test("capture: MCP App channel lifecycle", async ({ page }) => {
  test.setTimeout(45_000);
  await installMockBridge(page);

  const proxy = readFileSync(
    resolve(
      process.cwd(),
      "src-tauri/src/commands/mcp_apps_sandbox_proxy.html",
    ),
    "utf8",
  ).replace(
    "    /* BUZZ_MCP_APP_DEV_ORIGINS */",
    ',\n    "http://127.0.0.1:4173"',
  );
  await page.route("http://buzz-mcp-app.localhost/**", (route) =>
    route.fulfill({ body: proxy, contentType: "text/html", status: 200 }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await installMcpAppCommandMocks(page);
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/01-before-channel-app.png` });

  await page.getByTestId("channel-mcp-app-open-dialog").click();
  await page
    .getByRole("textbox", { name: "MCP server endpoint" })
    .fill(ENDPOINT);
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByText("Requested network access")).toBeVisible();
  await expect(
    page.getByText("Clipboard write", { exact: false }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/02-review-app-permissions.png` });

  await page.getByTestId("channel-mcp-app-add-tab").click();
  await page.getByRole("button", { name: "Project board" }).click();
  const app = page
    .frameLocator('iframe[title="Project board"]')
    .frameLocator("iframe");
  await expect(
    app.getByRole("heading", { name: "Project board" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/03-project-board-tab.png` });

  await app.getByRole("button", { name: "Share update" }).click();
  await expect(
    page.getByRole("heading", { name: "Post requested by a channel app?" }),
  ).toBeVisible();
  await expect(
    page.getByText("Move “Review API contract” to Review."),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/04-review-channel-post.png` });
});
