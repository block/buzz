import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const OUTDIR = "test-results/pr-3275-channel-apps";
const ENDPOINT = "https://runtime.example.test/mcp";
const BOARD_RESOURCE_URI = "ui://review/project-board";
const READER_RESOURCE_URI = "ui://review/signal-reader";
const REVIEW_RESOURCE_URI = "ui://review/artifact-review";
const SANDBOX_ORIGIN = "http://buzz-mcp-app.localhost";

const POLICY = {
  csp: {
    connectDomains: ["https://api.example.test"],
    resourceDomains: ["https://assets.example.test"],
    frameDomains: [],
    baseUriDomains: [],
  },
  requestedPermissions: {},
};

const SERVER = {
  serverId: "review-signal-reader",
  endpoint: ENDPOINT,
  name: "Signal workspace",
  version: "1.0.0",
  protocolVersion: "2025-11-25",
  tools: [
    {
      name: "open_project_board",
      title: "Project board",
      description: "Plan and track work across a shared project.",
      inputSchema: { type: "object", properties: {} },
      outputSchema: null,
      annotations: null,
      meta: {},
      uiResourceUri: BOARD_RESOURCE_URI,
      visibility: ["app", "model"],
    },
    {
      name: "prepare_brief",
      title: "Signal reader",
      description: "Read sources and prepare concise channel briefs.",
      inputSchema: {
        type: "object",
        properties: { storyId: { type: "string" } },
        required: ["storyId"],
      },
      outputSchema: null,
      annotations: null,
      meta: {},
      uiResourceUri: READER_RESOURCE_URI,
      visibility: ["app", "model"],
    },
    {
      name: "review_artifact",
      title: "Artifact review",
      description: "Review a concrete deliverable and record a decision.",
      inputSchema: { type: "object", properties: {} },
      outputSchema: null,
      annotations: null,
      meta: {},
      uiResourceUri: REVIEW_RESOURCE_URI,
      visibility: ["app", "model"],
    },
  ],
  resources: [
    {
      uri: BOARD_RESOURCE_URI,
      name: "Project board",
      title: "Project board",
      description: "A neutral project-work fixture.",
      mimeType: "text/html;profile=mcp-app",
      meta: {},
    },
    {
      uri: READER_RESOURCE_URI,
      name: "Signal reader",
      title: "Signal reader",
      description: "A neutral research-feed fixture.",
      mimeType: "text/html;profile=mcp-app",
      meta: {},
    },
    {
      uri: REVIEW_RESOURCE_URI,
      name: "Artifact review",
      title: "Artifact review",
      description: "A neutral decision-workspace fixture.",
      mimeType: "text/html;profile=mcp-app",
      meta: {},
    },
  ],
};

function staticAppBootstrap(name: string) {
  return `<script>
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
    (async()=>{
      await request("ui/initialize",{
        appInfo:{name:${JSON.stringify(name)},version:"1.0.0"},
        appCapabilities:{availableDisplayModes:["inline"]},
        protocolVersion:"2026-01-26"
      });
      send({jsonrpc:"2.0",method:"ui/notifications/initialized",params:{}});
    })();
  </script>`;
}

const BOARD_HTML = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <style>
    :root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;background:#1d1d24;color:#eee9d2}
    *{box-sizing:border-box}body{margin:0;background:#1d1d24}
    button{color:inherit;cursor:pointer;font:inherit}
    .shell{display:grid;grid-template-rows:62px 52px minmax(0,1fr);height:100vh;min-height:560px}
    header{align-items:center;border-bottom:1px solid #383840;display:flex;justify-content:space-between;padding:0 20px}
    h1{font-size:15px;margin:0}.eyebrow{color:#92909d;font-size:11px;margin-top:3px}
    .header-meta{align-items:center;color:#aaa7b2;display:flex;font-size:11px;gap:9px}
    .live{border:1px solid #3a3b43;border-radius:999px;padding:6px 9px}
    .live::before{background:#34c77b;border-radius:50%;content:"";display:inline-block;height:6px;margin-right:6px;width:6px}
    .toolbar{align-items:center;border-bottom:1px solid #32323a;display:flex;gap:8px;padding:0 20px}
    .view{background:#292932;border:1px solid #41414b;border-radius:7px;font-size:11px;padding:7px 10px}
    .quiet{color:#85838e;font-size:11px;margin-left:auto}.primary{background:#3478f6;border:1px solid #3478f6;border-radius:7px;color:white;font-size:11px;padding:7px 10px}
    .board{display:grid;gap:12px;grid-template-columns:repeat(4,minmax(220px,1fr));min-height:0;overflow:auto;padding:16px 18px 22px}
    .column{background:#202028;border:1px solid #34343d;border-radius:12px;display:flex;flex-direction:column;min-height:0;padding:12px}
    .column-head{align-items:center;display:flex;font-size:12px;font-weight:650;justify-content:space-between;margin:0 2px 10px}
    .count{background:#2b2b34;border-radius:999px;color:#918f9a;font-size:10px;font-weight:500;min-width:22px;padding:3px 7px;text-align:center}
    .cards{display:flex;flex-direction:column;gap:8px}
    .card{background:#292932;border:1px solid #3b3b45;border-radius:9px;padding:11px}
    .card.review{border-color:#645637}.card.done{opacity:.72}
    .label{color:#7fa8ff;font-size:9px;font-weight:700;letter-spacing:.1em;text-transform:uppercase}.label.review{color:#e2bd67}.label.done{color:#67c995}
    .title{font-size:12px;font-weight:650;line-height:1.35;margin:7px 0 9px}.meta{align-items:center;color:#85838e;display:flex;font-size:10px;gap:7px}
    .avatar{align-items:center;background:#3b3b48;border-radius:50%;color:#d7d3bf;display:inline-flex;font-size:8px;height:20px;justify-content:center;width:20px}
    .check{border-top:1px solid #3a3a43;color:#aaa7b2;font-size:10px;line-height:1.45;margin-top:10px;padding-top:8px}
    .progress{background:#393943;border-radius:999px;height:3px;margin-top:9px;overflow:hidden}.progress i{background:#6f9cff;display:block;height:100%}
  </style>
</head>
<body>
  <div class="shell">
    <header>
      <div><h1>Project board</h1><div class="eyebrow">Launch readiness · MCP App</div></div>
      <div class="header-meta"><span>8 items</span><span class="live">Synced</span></div>
    </header>
    <div class="toolbar">
      <button class="view">Board</button><button class="view">Milestones</button>
      <span class="quiet">Updated moments ago</span><button class="primary">Add work</button>
    </div>
    <main class="board">
      <section class="column">
        <div class="column-head"><span>Backlog</span><span class="count">2</span></div>
        <div class="cards">
          <article class="card"><div class="label">Platform</div><div class="title">Define the export contract</div><div class="meta"><span class="avatar">PL</span><span>Medium</span><span>Aug 4</span></div></article>
          <article class="card"><div class="label">Experience</div><div class="title">Audit empty and recovery states</div><div class="meta"><span class="avatar">DX</span><span>Low</span><span>Aug 6</span></div></article>
        </div>
      </section>
      <section class="column">
        <div class="column-head"><span>Active</span><span class="count">2</span></div>
        <div class="cards">
          <article class="card"><div class="label">Runtime</div><div class="title">Connect the live activity stream</div><div class="meta"><span class="avatar">RT</span><span>High</span><span>Today</span></div><div class="progress"><i style="width:68%"></i></div><div class="check">3 of 5 checks complete</div></article>
          <article class="card"><div class="label">Evidence</div><div class="title">Capture the onboarding proof</div><div class="meta"><span class="avatar">QA</span><span>Medium</span><span>Tomorrow</span></div></article>
        </div>
      </section>
      <section class="column">
        <div class="column-head"><span>Review</span><span class="count">2</span></div>
        <div class="cards">
          <article class="card review"><div class="label review">Approval required</div><div class="title">MCP App channel tabs</div><div class="meta"><span class="avatar">UI</span><span>Draft PR</span><span>#3275</span></div><div class="check">Security and interaction review requested</div></article>
          <article class="card"><div class="label">Architecture</div><div class="title">Validate the host boundary diagram</div><div class="meta"><span class="avatar">AR</span><span>2 comments</span></div></article>
        </div>
      </section>
      <section class="column">
        <div class="column-head"><span>Done</span><span class="count">2</span></div>
        <div class="cards">
          <article class="card done"><div class="label done">Complete</div><div class="title">Probe server capabilities</div><div class="meta"><span class="avatar">NW</span><span>Jul 29</span></div></article>
          <article class="card done"><div class="label done">Complete</div><div class="title">Require exact-message approval</div><div class="meta"><span class="avatar">SE</span><span>Jul 28</span></div></article>
        </div>
      </section>
    </main>
  </div>
  ${staticAppBootstrap("Project board")}
</body>
</html>`;

const REVIEW_HTML = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <style>
    :root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;background:#1d1d24;color:#eee9d2}
    *{box-sizing:border-box}body{margin:0;background:#1d1d24}
    button{color:inherit;cursor:pointer;font:inherit}
    .shell{display:grid;grid-template-rows:62px minmax(0,1fr);height:100vh;min-height:560px}
    header{align-items:center;border-bottom:1px solid #383840;display:flex;justify-content:space-between;padding:0 20px}
    h1{font-size:15px;margin:0}.eyebrow{color:#92909d;font-size:11px;margin-top:3px}
    .header-meta{align-items:center;display:flex;gap:8px}.badge{border:1px solid #665630;border-radius:999px;color:#e0bd6b;font-size:10px;padding:6px 9px}
    .workspace{display:grid;grid-template-columns:180px minmax(420px,1fr) 260px;min-height:0}
    nav,.decision{background:#202028;min-height:0;padding:18px 14px}.decision{border-left:1px solid #383840}nav{border-right:1px solid #383840}
    .label{color:#787681;font-size:9px;font-weight:700;letter-spacing:.12em;margin:0 7px 10px;text-transform:uppercase}
    .file{align-items:center;background:transparent;border:0;border-radius:7px;display:flex;font-size:11px;gap:8px;margin:2px 0;padding:8px;width:100%}.file.active{background:#2c2c35}.dot{background:#6f9cff;border-radius:2px;height:7px;width:7px}
    .document{min-height:0;overflow:auto;padding:30px 42px}.kicker{color:#7fa8ff;font-size:10px;font-weight:700;letter-spacing:.12em;text-transform:uppercase}
    h2{font-size:25px;letter-spacing:-.025em;margin:10px 0 8px}.byline{color:#85838e;font-size:11px}.lede{color:#c7c3cd;font-size:14px;line-height:1.65;margin:26px 0}
    h3{font-size:13px;margin:24px 0 8px}.copy{color:#aaa7b2;font-size:12px;line-height:1.65}
    .change{background:#222b27;border:1px solid #375044;border-radius:8px;color:#b7d8c5;font-family:ui-monospace,monospace;font-size:10px;line-height:1.55;margin-top:12px;padding:11px}
    .comment{border-left:2px solid #6f9cff;color:#aaa7b2;font-size:11px;line-height:1.55;margin:20px 0;padding:2px 0 2px 12px}
    .summary{border-bottom:1px solid #383840;padding-bottom:16px}.summary strong{display:block;font-size:12px;margin-bottom:5px}.summary span{color:#8e8c97;font-size:10px}
    .checks{display:grid;gap:8px;margin:16px 0}.check{align-items:center;color:#aaa7b2;display:flex;font-size:11px;gap:8px}.check i{background:#34c77b;border-radius:50%;height:7px;width:7px}
    .reviewers{display:flex;margin:14px 0}.avatar{align-items:center;background:#3b3b48;border:2px solid #202028;border-radius:50%;display:flex;font-size:8px;height:28px;justify-content:center;margin-left:-5px;width:28px}.avatar:first-child{margin-left:0}
    .actions{display:grid;gap:8px;margin-top:18px}.primary,.secondary{border-radius:8px;font-size:11px;padding:9px}.primary{background:#3478f6;border:1px solid #3478f6;color:white}.secondary{background:#292932;border:1px solid #41414b}
  </style>
</head>
<body>
  <div class="shell">
    <header>
      <div><h1>Artifact review</h1><div class="eyebrow">Channel apps proposal · revision 3</div></div>
      <div class="header-meta"><span class="badge">Decision required</span></div>
    </header>
    <main class="workspace">
      <nav aria-label="Artifacts">
        <div class="label">Artifacts</div>
        <button class="file active"><i class="dot"></i>proposal.md</button>
        <button class="file"><i class="dot"></i>architecture.mmd</button>
        <button class="file"><i class="dot"></i>security-notes.md</button>
        <button class="file"><i class="dot"></i>screenshots / 5</button>
        <div class="label" style="margin-top:24px">References</div>
        <button class="file">MCP Apps specification</button>
        <button class="file">Host capability policy</button>
      </nav>
      <article class="document">
        <div class="kicker">Proposal</div>
        <h2>Host task-specific interfaces inside a channel</h2>
        <div class="byline">Updated 24 minutes ago · 3 reviewers</div>
        <p class="lede">A channel can preserve the conversation while adding a structured interface for work that benefits from more than a message stream.</p>
        <h3>Decision</h3>
        <p class="copy">Adopt Streamable HTTP MCP Apps as the first packaging boundary. Keep transport, policy review, sandboxing and publication authority in the host.</p>
        <div class="change">+ Replace caller-supplied context with host-owned metadata.<br>+ Require approval for the exact attributed channel post.<br>+ Keep the embedded resource on an opaque origin.</div>
        <div class="comment"><strong>Security review</strong><br>The rendering resource never receives relay credentials. Network access stays within the policy reviewed at installation.</div>
        <h3>Evidence</h3>
        <p class="copy">The test fixture performs a real Streamable HTTP tool call and separately exercises installation, rendering, approval and one durable channel post.</p>
      </article>
      <aside class="decision">
        <div class="summary"><strong>Ready for decision</strong><span>2 approvals · 1 open question</span></div>
        <div class="checks">
          <div class="check"><i></i>Protocol tests passed</div>
          <div class="check"><i></i>Sandbox policy reviewed</div>
          <div class="check"><i></i>Neutral evidence captured</div>
        </div>
        <div class="label" style="margin:20px 0 8px">Reviewers</div>
        <div class="reviewers"><span class="avatar">SE</span><span class="avatar">UX</span><span class="avatar">PL</span></div>
        <div class="actions"><button class="primary">Approve proposal</button><button class="secondary">Request changes</button></div>
      </aside>
    </main>
  </div>
  ${staticAppBootstrap("Artifact review")}
</body>
</html>`;

const READER_HTML = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <style>
    :root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;background:#1d1d24;color:#eee9d2}
    *{box-sizing:border-box}body{margin:0;background:#1d1d24}
    button{color:inherit;cursor:pointer;font:inherit}
    .shell{display:grid;grid-template-rows:62px minmax(0,1fr);height:100vh;min-height:560px}
    header{align-items:center;border-bottom:1px solid #383840;display:flex;justify-content:space-between;padding:0 20px}
    h1{font-size:15px;margin:0}.eyebrow{color:#92909d;font-size:11px;margin-top:3px}
    .header-meta{align-items:center;color:#aaa7b2;display:flex;font-size:11px;gap:10px}
    .live{border:1px solid #3a3b43;border-radius:999px;padding:6px 9px}
    .live::before{background:#34c77b;border-radius:50%;content:"";display:inline-block;height:6px;margin-right:6px;width:6px}
    .reader{display:grid;grid-template-columns:172px minmax(260px,340px) minmax(380px,1fr);min-height:0}
    nav,.stories{border-right:1px solid #383840;min-height:0;overflow:auto}
    nav{padding:18px 12px}
    .nav-label{color:#787681;font-size:10px;font-weight:700;letter-spacing:.11em;margin:0 8px 9px;text-transform:uppercase}
    .source{align-items:center;background:transparent;border:0;border-radius:8px;display:flex;font-size:12px;justify-content:space-between;margin:2px 0;padding:8px;width:100%}
    .source:hover,.source.active{background:#2a2a33}.source span:last-child{color:#817f8a;font-variant-numeric:tabular-nums}
    .source-dot{background:#6f9cff;border-radius:50%;display:inline-block;height:6px;margin-right:8px;width:6px}
    .stories{padding:10px}
    .story{background:transparent;border:1px solid transparent;border-radius:10px;display:block;margin-bottom:4px;padding:12px;text-align:left;width:100%}
    .story:hover{background:#22222a}.story.active{background:#272730;border-color:#3d3d48}
    .story-meta{color:#85838e;font-size:10px;letter-spacing:.04em;margin-bottom:7px;text-transform:uppercase}
    .story-title{font-size:13px;font-weight:650;line-height:1.35}
    .story-dek{color:#a7a4af;font-size:11px;line-height:1.45;margin-top:6px}
    .unread .story-title::before{background:#6f9cff;border-radius:50%;content:"";display:inline-block;height:6px;margin:0 7px 1px 0;width:6px}
    article{min-height:0;overflow:auto;padding:26px 30px 34px}
    .kicker{color:#7fa8ff;font-size:10px;font-weight:700;letter-spacing:.12em;text-transform:uppercase}
    h2{font-size:24px;letter-spacing:-.025em;line-height:1.15;margin:10px 0 12px;max-width:720px}
    .byline{color:#85838e;font-size:11px}.summary{color:#c7c3cd;font-size:14px;line-height:1.65;margin:26px 0;max-width:690px}
    .evidence{border-left:2px solid #4c78d4;color:#aaa7b2;font-size:12px;line-height:1.55;margin:24px 0;padding:2px 0 2px 14px}
    .brief{background:#23232b;border:1px solid #3a3a45;border-radius:10px;margin-top:24px;padding:14px}
    .brief-label{color:#85838e;font-size:10px;font-weight:700;letter-spacing:.1em;text-transform:uppercase}
    #brief-text{font-size:12px;line-height:1.55;margin:8px 0 0;white-space:pre-wrap}
    .actions{align-items:center;display:flex;gap:8px;margin-top:18px}
    .primary,.secondary{border-radius:8px;font-size:11px;padding:8px 11px}
    .primary{background:#3478f6;border:1px solid #3478f6;color:white}.secondary{background:#292932;border:1px solid #41414b}
    button:disabled{cursor:default;opacity:.45}
    #status{color:#85838e;font-size:11px;margin-left:4px}
  </style>
</head>
<body>
  <div class="shell">
    <header>
      <div><h1>Signal reader</h1><div class="eyebrow">Research feed · MCP App</div></div>
      <div class="header-meta"><span>12 unread</span><span class="live">Connected</span></div>
    </header>
    <main class="reader">
      <nav aria-label="Sources">
        <div class="nav-label">Library</div>
        <button class="source active"><span><i class="source-dot"></i>All signals</span><span>12</span></button>
        <button class="source"><span>Product</span><span>5</span></button>
        <button class="source"><span>Research</span><span>4</span></button>
        <button class="source"><span>Security</span><span>3</span></button>
      </nav>
      <section class="stories" aria-label="Stories">
        <button class="story active unread">
          <div class="story-meta">Protocol notes · 18 min</div>
          <div class="story-title">Interactive tools move into the conversation</div>
          <div class="story-dek">A host-controlled pattern for rendering task-specific interfaces.</div>
        </button>
        <button class="story unread">
          <div class="story-meta">Systems journal · 42 min</div>
          <div class="story-title">Where agents need explicit approval boundaries</div>
          <div class="story-dek">A field guide to separating context from authority.</div>
        </button>
        <button class="story unread">
          <div class="story-meta">Research desk · 1 hr</div>
          <div class="story-title">Shared feeds as team memory</div>
          <div class="story-dek">What changes when triage happens in a durable channel.</div>
        </button>
        <button class="story">
          <div class="story-meta">Security review · Yesterday</div>
          <div class="story-title">Sandboxing untrusted interface resources</div>
          <div class="story-dek">Opaque origins, reviewed policy and credential confinement.</div>
        </button>
      </section>
      <article>
        <div class="kicker">Protocol notes</div>
        <h2>Interactive tools move into the conversation</h2>
        <div class="byline">Source captured 18 minutes ago · 6 minute read</div>
        <p class="summary">The useful boundary is not chat versus interface. It is who owns context, who can act, and where the resulting work becomes visible. A task-specific view can stay inside the channel while the host retains approval and publication authority.</p>
        <div class="evidence">The embedded view does not receive channel credentials. It can call tools declared by its MCP server. A request to post back into the conversation remains subject to Buzz review.</div>
        <div class="brief" aria-live="polite">
          <div class="brief-label">Channel brief</div>
          <p id="brief-text">Prepare a brief before sharing this signal.</p>
        </div>
        <div class="actions">
          <button class="primary" id="prepare">Prepare brief</button>
          <button class="secondary" disabled id="share">Share to channel</button>
          <span id="status"></span>
        </div>
      </article>
    </main>
  </div>
  <script>
    const pending = new Map();
    let requestId = 0;
    let preparedBrief = "";
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
    document.getElementById("prepare").addEventListener("click",async(event)=>{
      event.currentTarget.disabled=true;
      try{
        document.getElementById("status").textContent="Preparing…";
        const result=await request("tools/call",{name:"prepare_brief",arguments:{storyId:"interactive-tools"}});
        preparedBrief=result?.content?.find((item)=>item?.type==="text")?.text||"";
        document.getElementById("brief-text").textContent=preparedBrief;
        document.getElementById("share").disabled=!preparedBrief;
        document.getElementById("status").textContent="Ready";
      }finally{event.currentTarget.disabled=false}
    });
    document.getElementById("share").addEventListener("click",async(event)=>{
      event.currentTarget.disabled=true;
      try{
        document.getElementById("status").textContent="Waiting for Buzz approval…";
        await request("ui/message",{role:"user",content:[{type:"text",text:preparedBrief}]});
        document.getElementById("status").textContent="Shared";
      }catch{
        document.getElementById("status").textContent="Not shared";
      }finally{event.currentTarget.disabled=false}
    });
    (async()=>{
      await request("ui/initialize",{
        appInfo:{name:"Signal reader",version:"1.0.0"},
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
    ({ appHtmlByResource, policy, sandboxOrigin, server }) => {
      const testWindow = window as Window & {
        __MCP_APP_TOOL_CALLS__?: Record<string, unknown>[];
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
      testWindow.__MCP_APP_TOOL_CALLS__ = [];
      testWindow.__TAURI_INTERNALS__.invoke = async (command, payload) => {
        switch (command) {
          case "connect_mcp_app_server":
            return server;
          case "inspect_mcp_app_resource":
            return policy;
          case "prepare_mcp_app_view": {
            const uri = String(payload?.uri ?? "");
            const html = appHtmlByResource[uri];
            if (!html) throw new Error(`Unknown MCP App resource: ${uri}`);
            const slug = uri.split("/").at(-1) ?? "app";
            return {
              viewId: `${slug}-view`,
              sandboxUrl: `${sandboxOrigin}/${slug}`,
              html,
              csp: policy.csp,
              requestedPermissions: policy.requestedPermissions,
            };
          }
          case "call_mcp_app_tool":
            testWindow.__MCP_APP_TOOL_CALLS__?.push(payload ?? {});
            return {
              content: [
                {
                  type: "text",
                  text:
                    "MCP Apps let a channel host task-specific interfaces " +
                    "without giving the embedded view channel credentials. " +
                    "Buzz retains approval and publication authority.",
                },
              ],
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
      appHtmlByResource: {
        [BOARD_RESOURCE_URI]: BOARD_HTML,
        [READER_RESOURCE_URI]: READER_HTML,
        [REVIEW_RESOURCE_URI]: REVIEW_HTML,
      },
      policy: POLICY,
      sandboxOrigin: SANDBOX_ORIGIN,
      server: SERVER,
    },
  );
}

async function installChannelApp(
  page: import("@playwright/test").Page,
  title: string,
  capturePolicyReview = false,
  activate = true,
) {
  await page.getByTestId("channel-mcp-app-open-dialog").click();
  await page
    .getByRole("textbox", { name: "MCP server endpoint" })
    .fill(ENDPOINT);
  await page.getByRole("button", { name: "Connect" }).click();
  await page
    .getByTestId("channel-mcp-app-tool")
    .filter({ hasText: title })
    .click();
  await expect(page.getByText("Requested network access")).toBeVisible();
  if (capturePolicyReview) {
    await waitForAnimations(page);
    await page.screenshot({
      path: `${OUTDIR}/02-review-app-permissions.png`,
    });
  }
  await page.getByTestId("channel-mcp-app-add-tab").click();
  if (activate) {
    await page.getByRole("button", { name: title, exact: true }).click();
  }
}

test("capture: MCP App channel surfaces and interaction lifecycle", async ({
  page,
}) => {
  test.setTimeout(45_000);
  await page.emulateMedia({ colorScheme: "dark" });
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

  await installChannelApp(page, "Project board", true);
  const board = page
    .frameLocator('iframe[title="Project board"]')
    .frameLocator("iframe");
  await expect(
    board.getByRole("heading", { name: "Project board" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/03-project-board.png` });

  await installChannelApp(page, "Signal reader", false, false);
  await expect(
    board.getByRole("heading", { name: "Project board" }),
  ).toBeVisible();
  const boardToolCalls = await page.evaluate(
    () =>
      (
        window as Window & {
          __MCP_APP_TOOL_CALLS__?: Record<string, unknown>[];
        }
      ).__MCP_APP_TOOL_CALLS__?.filter(
        (call) => call.name === "open_project_board",
      ) ?? [],
  );
  expect(boardToolCalls).toHaveLength(1);
  await page
    .getByRole("button", { name: "Signal reader", exact: true })
    .click();
  const app = page
    .frameLocator('iframe[title="Signal reader"]')
    .frameLocator("iframe");
  await expect(
    app.getByRole("heading", { name: "Signal reader" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/04-signal-reader.png` });

  await app.getByRole("button", { name: "Prepare brief" }).click();
  await expect(app.getByText("Buzz retains approval")).toBeVisible();
  const toolCalls = await page.evaluate(
    () =>
      (
        window as Window & {
          __MCP_APP_TOOL_CALLS__?: Record<string, unknown>[];
        }
      ).__MCP_APP_TOOL_CALLS__ ?? [],
  );
  const readerToolCalls = toolCalls.filter(
    (call) => call.name === "prepare_brief",
  );
  const appToolCalls = readerToolCalls.filter((call) => call.caller === "app");
  const hostToolCalls = readerToolCalls.filter(
    (call) => call.caller === "host",
  );
  expect(hostToolCalls).toHaveLength(1);
  expect(appToolCalls).toHaveLength(1);
  expect(appToolCalls[0]).toMatchObject({
    name: "prepare_brief",
    caller: "app",
    arguments: { storyId: "interactive-tools" },
    context: {
      channelRef: expect.any(String),
      installationRef: expect.any(String),
    },
  });

  await app.getByRole("button", { name: "Share to channel" }).click();
  await expect(
    page.getByRole("heading", { name: "Post requested by a channel app?" }),
  ).toBeVisible();
  await expect(
    page.getByText("Buzz retains approval and publication authority."),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/05-review-channel-post.png` });

  await page.getByRole("button", { name: "Post to channel" }).click();
  await expect(
    page.getByRole("heading", { name: "Post requested by a channel app?" }),
  ).not.toBeVisible();
  await expect(app.getByText("Shared", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Chat", exact: true }).click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("MCP App · Signal reader");
  await expect(
    timeline.locator("[data-message-id]").filter({
      hasText: "Buzz retains approval and publication authority.",
    }),
  ).toHaveCount(1);
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/06-posted-to-channel.png` });

  await installChannelApp(page, "Artifact review");
  const review = page
    .frameLocator('iframe[title="Artifact review"]')
    .frameLocator("iframe");
  await expect(
    review.getByRole("heading", { name: "Artifact review" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${OUTDIR}/07-artifact-review.png` });
});
