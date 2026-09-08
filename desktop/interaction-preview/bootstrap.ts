/** Local demonstration only. Production builds never import this entry point. */
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../tests/helpers/features";

const relay = "a".repeat(64);
const viewer = "deadbeef".repeat(8);
const relayUrl = "ws://localhost:3000";
const communityId = "e2e-default-community";
// Select the repository's mock browser bridge. No real identity is loaded.
window.__BUZZ_E2E__ = { mode: "mock", mock: { relaySelf: relay } };
localStorage.setItem(
  "buzz-communities",
  JSON.stringify([
    {
      id: communityId,
      name: "Interaction preview",
      relayUrl,
      pubkey: viewer,
      addedAt: new Date().toISOString(),
    },
  ]),
);
localStorage.setItem("buzz-active-community-id", communityId);
localStorage.setItem(`buzz-onboarding-complete.v1:${viewer}`, "true");
localStorage.setItem(
  `buzz-welcome-channel-ensured.v2:${encodeURIComponent(relayUrl)}:${viewer}`,
  "true",
);
localStorage.setItem(
  FEATURE_OVERRIDES_STORAGE_KEY,
  JSON.stringify({ interactions: true }),
);
await import("../src/main");

const banner = document.createElement("aside");
banner.setAttribute("aria-label", "Preview information");
banner.style.cssText =
  "position:fixed;right:16px;bottom:16px;z-index:99999;max-width:360px;padding:12px 16px;border:1px solid #908bfb;border-radius:12px;background:#19182a;color:white;font:13px/1.5 system-ui;box-shadow:0 8px 30px #0004";
banner.textContent =
  "Interaction preview · Simulated data. Open #engineering to try buttons, a form and a poll. No decisions are sent to a real relay. ";
const reset = document.createElement("button");
reset.textContent = "Reset examples";
reset.style.cssText = "text-decoration:underline;cursor:pointer";
reset.onclick = () => location.reload();
banner.append(reset);
document.body.append(banner);

const deadline = Math.floor(Date.now() / 1000) + 3600;
const examples = [
  {
    id: "a1".padStart(64, "0"),
    projection: "a2".padStart(64, "0"),
    type: "buttons",
    text: "Render **The Door**? 12 stills, 2 hero clips, about 55 GPU minutes.",
    options: [
      ["approve", "Approve", "primary"],
      ["revise", "Send back"],
      ["deny", "Deny", "danger"],
    ],
    fields: [["field", "note", "Note for the writer", "text", "optional"]],
  },
  {
    id: "b1".padStart(64, "0"),
    projection: "b2".padStart(64, "0"),
    type: "form",
    text: "What should the next episode include?",
    options: [],
    fields: [
      ["field", "title", "Episode title", "text", "required"],
      ["field", "length", "Length", "select", "required"],
      ["optsel", "length", "60s", "60 seconds"],
      ["optsel", "length", "6min", "6 minutes"],
      ["field", "ready", "Ready to review?", "boolean", "required"],
    ],
  },
  {
    id: "c1".padStart(64, "0"),
    projection: "c2".padStart(64, "0"),
    type: "poll",
    text: "Which thumbnails should we test? Choose up to two; you can change your answer until this poll closes.",
    options: [
      ["door", "The door"],
      ["key", "The key"],
      ["hall", "The hallway"],
    ],
    fields: [],
  },
];
const states = new Map<
  string,
  {
    revision: number;
    status: string;
    close_reason: string | null;
    winner: string | null;
    tally: Record<string, number>;
    responders: unknown[];
  }
>();
let seeded = false;
let processed = 0;
let serial = 100;
function emitState(id: string) {
  window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
    channelName: "engineering",
    id: "beef".repeat(8) + (++serial).toString(16).padStart(32, "0"),
    kind: 39010,
    pubkey: relay,
    content: JSON.stringify({ version: 1, ...states.get(id) }),
    extraTags: [["d", id]],
  });
}
setInterval(() => {
  const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
  if (
    !emit ||
    !window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "engineering",
    })
  )
    return;
  if (!seeded) {
    seeded = true;
    for (const example of examples) {
      emit({
        channelName: "engineering",
        id: example.id,
        kind: 40010,
        content: example.text,
        extraTags: [
          ["itype", example.type],
          ["closes", example.type === "poll" ? "manual" : "first"],
          ["deadline", String(deadline)],
          ["min", example.type === "form" ? "0" : "1"],
          [
            "max",
            example.type === "form" ? "0" : example.type === "poll" ? "2" : "1",
          ],
          ...example.options.map((o) => ["opt", ...o]),
          ...example.fields,
        ],
      });
      emit({
        channelName: "engineering",
        id: example.projection,
        kind: 9,
        pubkey: relay,
        content: `${example.text}\n${example.options.map((o) => `${o[0]}: ${o[1]}`).join("\n")}\nEnable Interaction cards in Experimental Features to try the controls.`,
        extraTags: [["interaction", example.id]],
      });
      states.set(example.id, {
        revision: 0,
        status: "open",
        close_reason: null,
        winner: null,
        tally: Object.fromEntries(example.options.map((o) => [o[0], 0])),
        responders: [],
      });
      emitState(example.id);
    }
  }
  // Demonstration responses are simulated. No signing keys, network calls or
  // replacement for the Rust relay's acceptance/authorization rules live here.
  const events = window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [];
  for (const event of events.slice(processed)) {
    const id = event.tags.find((t) => t[0] === "e" && t[3] === "prompt")?.[1];
    const example = examples.find((e) => e.id === id);
    const state = id && states.get(id);
    if (!example || !state || state.status === "closed") continue;
    const choices = event.tags
      .filter((t) => t[0] === "choice")
      .map((t) => t[1]);
    state.revision++;
    if (event.kind === 40012 || example.type !== "poll") {
      state.status = "closed";
      state.close_reason = event.kind === 40012 ? "manual" : "first";
    }
    if (event.kind === 40011) {
      state.tally = Object.fromEntries(
        example.options.map((o) => [o[0], Number(choices.includes(o[0]))]),
      );
      state.responders = [
        {
          pubkey: viewer,
          event_id:
            "beef".repeat(8) + (++serial).toString(16).padStart(32, "0"),
          created_at: event.createdAt ?? Math.floor(Date.now() / 1000),
          choices,
        },
      ];
      if (example.type === "buttons") state.winner = choices[0] ?? null;
    }
    emitState(example.id);
  }
  processed = events.length;
}, 300);
