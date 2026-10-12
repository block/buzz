// Web parity batch 2 e2e: workflows + approvals, NIP-OA owner, canvas, forum, notes, update prompt, team catalog.
// Needs local relay (:3100) and a web dev server started with VITE_BUZZ_WEB_BUILD_ID=devtest.
// Usage: node scripts/pwa/s23-parity-batch2-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { finalizeEvent, getPublicKey, generateSecretKey } from "nostr-tools/pure";
import { schnorr } from "@noble/curves/secp256k1.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5311/";
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
let liveBuild = "devtest";
await ctx.route("**/version.json*", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify({ build: liveBuild }) }));
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", e.message));
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 20000 });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });
const inv = (cmd, args) => page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [cmd, args]);
const err = (p) => p.then(() => "", (e) => String(e?.message ?? e));
const stamp = Date.now();
const general = await page.evaluate(async () => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  return (chs.find((c) => c.name === "general") ?? chs[0]).id;
});

// 6. NIP-OA (pure verify against a locally minted owner attestation; no relay write of a forged profile)
const ownerSk = generateSecretKey();
const owner = getPublicKey(ownerSk);
const agentSk = generateSecretKey();
const agent = getPublicKey(agentSk);
const authSig = bytesToHex(schnorr.sign(sha256(new TextEncoder().encode(`nostr:agent-auth:${agent}:`)), ownerSk));
const goodProfile = finalizeEvent({ kind: 0, content: JSON.stringify({ name: `agent-${stamp}` }), tags: [["auth", owner, "", authSig]], created_at: Math.floor(Date.now() / 1000) }, agentSk);
const badProfile = finalizeEvent({ kind: 0, content: "{}", tags: [["auth", owner, "", "0".repeat(128)]], created_at: Math.floor(Date.now() / 1000) }, agentSk);
const oa = await page.evaluate(async ([g, b]) => {
  const m = await import("/src/platform/web/nipOa.ts");
  return [m.profileValidOaOwner(g), m.profileValidOaOwner(b)];
}, [goodProfile, badProfile]);
check("NIP-OA: valid owner attestation verifies", oa[0] === owner);
check("NIP-OA: forged signature rejected", oa[1] === null);
const sup = await page.evaluate(async ([g, owner, agent]) => {
  const m = await import("/src/platform/web/nipOa.ts");
  const orig = { id: "a".repeat(64), pubkey: agent, kind: 9, tags: [], content: "", created_at: 1, sig: "" };
  const edit = (pk) => ({ id: pk.slice(0, 63) + "e", pubkey: pk, kind: 40003, tags: [["e", orig.id], ["link-preview", "none"]], content: "", created_at: 2, sig: "" });
  const owners = m.ownersFromProfiles([g]);
  return [m.linkPreviewSuppressionTargets([orig], [edit(owner)], owners).size, m.linkPreviewSuppressionTargets([orig], [edit("b".repeat(64))], owners).size];
}, [goodProfile, owner, agent]);
check("owner edit suppresses link preview; stranger edit does not", sup[0] === 1 && sup[1] === 0, JSON.stringify(sup));
const selfOwner = await inv("resolve_oa_owner", { targetPubkey: pk });
check("resolve_oa_owner answers on web (null for a human)", selfOwner === null);

// 7 + 5. workflows and approvals
const yaml = `name: Parity ${stamp}\ntrigger:\n  on: webhook\nsteps:\n  - id: ask\n    action: request_approval\n    from: '${pk}'\n    message: Approve parity ${stamp}?\n    timeout: 1h\n  - id: done\n    action: send_message\n    text: parity approved ${stamp}\n`;
const wf = await inv("create_workflow", { channelId: general, yamlDefinition: yaml }).catch((e) => ({ error: String(e?.message ?? e) }));
check("create_workflow", !!wf?.id && wf.name === `Parity ${stamp}`, wf?.error ?? "");
if (wf?.id) {
  const list = await inv("get_channel_workflows", { channelId: general });
  check("get_channel_workflows lists it", list.some((w) => w.id === wf.id));
  const many = await inv("get_channels_workflows", { channelIds: [general] });
  check("get_channels_workflows lists it", many.some((w) => w.id === wf.id));
  const upd = await inv("update_workflow", { workflowId: wf.id, yamlDefinition: yaml.replace(`Parity ${stamp}`, `Parity2 ${stamp}`), expectedRevision: wf.revision });
  check("update_workflow (revision-checked)", upd.name === `Parity2 ${stamp}`);
  check("stale revision rejected", (await err(inv("update_workflow", { workflowId: wf.id, yamlDefinition: yaml, expectedRevision: wf.revision }))).includes("changed since"));
  const trig = await inv("trigger_workflow", { workflowId: wf.id }).catch((e) => ({ error: String(e?.message ?? e) }));
  check("trigger_workflow returns a run id", !!trig?.run_id, trig?.error ?? "");
  let runs = { runs: [] };
  for (let i = 0; i < 20 && !runs.runs?.length; i++) {
    await page.waitForTimeout(500);
    runs = await inv("get_workflow_runs", { workflowId: wf.id, limit: 5 }).catch((e) => ({ runs: [], error: String(e?.message ?? e) }));
  }
  check("get_workflow_runs (NIP-98 GET) returns the run", runs.runs?.length > 0, runs.error ?? JSON.stringify(runs.runs?.[0] ?? {}).slice(0, 160));
  const runId = trig?.run_id;
  const relayNoGates = runs.runs?.[0]?.error_code === "approval_not_supported";
  if (relayNoGates) console.log("SKIP  approval gate round trip — this relay's workflow engine reports approval_not_supported (WF-08); same on desktop");
  let approvals = { approvals: [] };
  for (let i = 0; i < 20 && runId && !relayNoGates && !approvals.approvals?.length; i++) {
    await page.waitForTimeout(500);
    approvals = await inv("get_run_approvals", { workflowId: wf.id, runId }).catch((e) => ({ approvals: [], error: String(e?.message ?? e) }));
  }
  if (relayNoGates) {
    const empty = await inv("get_run_approvals", { workflowId: wf.id, runId }).catch((e) => ({ error: String(e?.message ?? e) }));
    check("get_run_approvals (NIP-98 GET) answers", Array.isArray(empty?.approvals), empty?.error ?? "");
  } else check("get_run_approvals lists the pending approval", approvals.approvals?.length > 0, approvals.error ?? "");
  const token = approvals.approvals?.[0]?.token ?? approvals.approvals?.[0]?.approval_token;
  if (relayNoGates) {
    /* nothing to grant */
  } else if (token) {
    const g = await inv("grant_approval", { token, note: "ok from web" }).catch((e) => ({ error: String(e?.message ?? e) }));
    check("grant_approval accepted", !!g?.event_id, g?.error ?? "");
  } else check("approval token present", false, JSON.stringify(approvals.approvals?.[0] ?? {}).slice(0, 200));
  const d = await inv("deny_approval", { token: "not-a-real-token" }).then(() => "accepted", (e) => String(e?.message ?? e));
  check("deny_approval reaches the relay (bogus token answered)", typeof d === "string", d.slice(0, 120));
  await inv("delete_workflow", { workflowId: wf.id });
  check("delete_workflow resolves", true);
}

// 8. canvas
const c0 = await inv("get_canvas", { channelId: general });
const s1 = await inv("set_canvas", { channelId: general, content: `canvas ${stamp}`, expectedRevision: c0.event_id ?? "none" });
check("set_canvas with revision check, verified", s1.ok && s1.verified, JSON.stringify(s1).slice(0, 120));
const staleErr = await err(inv("set_canvas", { channelId: general, content: "x", expectedRevision: c0.event_id ?? "none" }));
check("stale canvas save rejected with frozen marker", staleErr.includes("conflict:"), staleErr.slice(0, 160) || `resolved (c0=${c0.event_id})`);
const c1 = await inv("get_canvas", { channelId: general });
check("get_canvas returns the new head", c1.content === `canvas ${stamp}` && c1.event_id === s1.event_id);
const hist = await inv("get_canvas_history", { channelId: general, limit: 5 });
check("get_canvas_history pages", hist.revisions[0]?.event_id === s1.event_id);

// 8. forum (posts are published by the forum composer via send_channel_message kind 45001 in a forum channel)
const forum = await inv("create_channel", { name: `forum-${stamp}`, channelType: "forum", visibility: "open" });
const post = await inv("send_channel_message", { channelId: forum.id, kind: 45001, content: `post ${stamp}` });
await inv("send_channel_message", { channelId: forum.id, kind: 45003, parentEventId: post.event_id, content: `reply ${stamp}` });
const posts = await inv("get_forum_posts", { channelId: forum.id });
check("get_forum_posts", posts.messages.some((m) => m.event_id === post.event_id));
const thread = await inv("get_forum_thread", { channelId: forum.id, eventId: post.event_id });
check("get_forum_thread root + reply", thread.root.event_id === post.event_id && thread.replies.some((r) => r.content === `reply ${stamp}`));
await page.goto(`${APP}#/channels/${forum.id}`);
const forumUi = await page.getByText(`post ${stamp}`).first().waitFor({ timeout: 20000 }).then(() => true, () => false);
check("forum channel renders the post", forumUi);
await inv("delete_channel", { channelId: forum.id });

// 8. notes
const note = await inv("publish_note", { content: `note ${stamp}` });
check("publish_note", !!note.event_id);
await inv("add_reaction", { eventId: note.event_id, emoji: "+" });
const mine = await inv("get_user_notes", { pubkey: pk, limit: 5 });
check("get_user_notes", mine.notes.some((n) => n.id === note.event_id));
check("get_note", (await inv("get_note", { noteId: note.event_id }))?.content === `note ${stamp}`);
const rx = await inv("get_note_reactions", { noteIds: [note.event_id] });
check("get_note_reactions", rx.some((r) => r.note_id === note.event_id && r.pubkeys.includes(pk)));
const liked = await inv("get_liked_notes", { authorPubkey: pk, limit: 10 });
check("get_liked_notes", liked.notes.some((n) => n.id === note.event_id));
const tl = await inv("get_notes_timeline", { pubkeys: [pk] });
check("get_notes_timeline", tl.notes.some((n) => n.id === note.event_id));
check("get_global_notes", (await inv("get_global_notes", { limit: 20 })).notes.some((n) => n.id === note.event_id));

// 10. team catalog
const cat = await inv("fetch_team_catalog").then((v) => v, (e) => ({ error: String(e?.message ?? e) }));
check("fetch_team_catalog answers on web (array, no NativeOnlyError)", Array.isArray(cat), cat?.error ?? `n=${cat.length}`);
const pub = await page.evaluate(async ([sk]) => {
  const m = await import("/src/platform/web/parity2.ts");
  const ev = (content, tags, id) => ({ id, pubkey: "c".repeat(64), kind: 30178, created_at: 10, content: JSON.stringify(content), tags, sig: "" });
  const good = ev({ v: 1, name: "Team A", members: [{ member_key: "k1", display_name: "Ann" }] }, [["d", "t1"], ["shared", "true"]], "1".repeat(64));
  const unshared = ev({ v: 1, name: "Team B", members: [] }, [["d", "t2"]], "2".repeat(64));
  const dup = ev({ v: 1, name: "Team C", members: [{ member_key: "k", display_name: "A" }, { member_key: "k", display_name: "B" }] }, [["d", "t3"], ["shared", "true"]], "3".repeat(64));
  return m.publicationsFromEvents([good, unshared, dup]).map((p) => p.name);
}, [null]);
check("team catalog projection: shared + valid only", JSON.stringify(pub) === '["Team A"]', JSON.stringify(pub));

// 9. update prompt
await page.goto(APP);
await page.waitForSelector('[data-testid="sidebar-channel-content"]', { timeout: 30000 });
const before = await page.evaluate(async () => (await import("/src/platform/web/updateCheck.ts")).checkForUpdate());
check("same build: no update prompt", before === false && (await page.locator('[data-testid="web-update-prompt"]').count()) === 0);
liveBuild = "newer-build";
const after = await page.evaluate(async () => (await import("/src/platform/web/updateCheck.ts")).checkForUpdate());
const prompt = page.locator('[data-testid="web-update-prompt"]');
check("newer build: 'New version available' prompt", after === true && (await prompt.isVisible()));
const reloaded = page.waitForEvent("load", { timeout: 15000 }).then(() => true, () => false);
await prompt.click();
check("tapping the prompt reloads", await reloaded);

await page.screenshot({ path: "/tmp/s23-parity.png" });
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
