// Web observer e2e: decrypt_observer_event + build_observer_control_event web handlers (mock NIP-07 signer with nip44).
// Needs local relay (:3100) and a web dev server. Usage: node scripts/pwa/s13-observer-e2e.mjs [appUrl] (default http://127.0.0.1:5199/)
import { chromium } from "playwright";
import { finalizeEvent, generateSecretKey, getPublicKey, nip44 } from "nostr-tools";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
const ctx = await browser.newContext();
const { pk } = await installMockExtension(ctx, TYLER);
await ctx.addInitScript(({ relay, pk }) => {
  if (!localStorage.getItem("buzz-communities")) {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
  }
}, { relay: RELAY_WS, pk });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", e.message));
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 20000 });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });
const inv = (cmd, args) => page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [cmd, args]);

// An "agent" key encrypts a telemetry frame to the owner (the signer) exactly like the harness does.
const agentSk = generateSecretKey();
const agentPk = getPublicKey(agentSk);
const payload = { type: "turn_started", turnId: "turn-1", channelId: "chan-xyz", seq: 7 };
const frame = (sk, content) =>
  finalizeEvent({ kind: 24200, created_at: Math.floor(Date.now() / 1000), content, tags: [["p", pk], ["agent", getPublicKey(sk)], ["frame", "telemetry"]] }, sk);
const ev = frame(agentSk, nip44.v2.encrypt(JSON.stringify(payload), nip44.v2.utils.getConversationKey(agentSk, pk)));

const out = await inv("decrypt_observer_event", { eventJson: JSON.stringify(ev) });
check("decrypt_observer_event returns the agent's payload", JSON.stringify(out) === JSON.stringify(payload), JSON.stringify(out));
const tampered = { ...ev, content: ev.content.slice(0, -4) + "AAAA" };
const t = await inv("decrypt_observer_event", { eventJson: JSON.stringify(tampered) }).then(() => "ok", (e) => String(e?.message ?? e));
check("tampered event rejected (id/signature check)", /invalid ID or signature/.test(t), t);
const short = frame(agentSk, "not encrypted");
const s = await inv("decrypt_observer_event", { eventJson: JSON.stringify(short) }).then(() => "ok", (e) => String(e?.message ?? e));
check("short/non-NIP-44 content rejected", /invalid NIP-44 ciphertext length/.test(s), s);
const other = generateSecretKey();
const wrong = frame(other, nip44.v2.encrypt("{}".padEnd(200, " "), nip44.v2.utils.getConversationKey(other, getPublicKey(generateSecretKey()))));
const w = await inv("decrypt_observer_event", { eventJson: JSON.stringify(wrong) }).then(() => "ok", (e) => String(e?.message ?? e));
check("frame not addressed to owner fails to decrypt", w !== "ok", w.slice(0, 80));
check("no NativeOnlyError text", !/not available in the web app/.test(t + s + w));

// control frame: owner -> agent, agent can decrypt it
const ctlJson = await inv("build_observer_control_event", { agentPubkey: agentPk, payload: { type: "cancel_turn", turnId: "turn-1" } });
const ctl = JSON.parse(ctlJson);
check("control event is kind 24200 signed by owner", ctl.kind === 24200 && ctl.pubkey === pk && typeof ctl.sig === "string");
const tag = (n) => ctl.tags.find((x) => x[0] === n)?.[1];
check("control tags p/agent=agent, frame=control", tag("p") === agentPk && tag("agent") === agentPk && tag("frame") === "control");
const plain = JSON.parse(nip44.v2.decrypt(ctl.content, nip44.v2.utils.getConversationKey(agentSk, pk)));
check("agent can decrypt the control payload", plain.type === "cancel_turn" && plain.turnId === "turn-1");
const badAgent = await inv("build_observer_control_event", { agentPubkey: "nothex", payload: {} }).then(() => "ok", (e) => String(e?.message ?? e));
check("invalid agent pubkey rejected", /invalid agent pubkey/.test(badAgent), badAgent);
check("no error boundary", (await page.getByText("Something went wrong").count()) === 0);
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
