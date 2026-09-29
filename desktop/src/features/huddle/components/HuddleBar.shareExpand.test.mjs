import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const src = fs.readFileSync(
  new URL("./HuddleBar.tsx", import.meta.url),
  "utf8",
);
const css = fs.readFileSync(
  new URL("../../../shared/styles/globals/components.css", import.meta.url),
  "utf8",
);
const participantListSrc = fs.readFileSync(
  new URL("./ParticipantList.tsx", import.meta.url),
  "utf8",
);

test("HuddleBar expands share into a shell stage portal, not OS fullscreen", () => {
  assert.match(src, /createPortal/);
  assert.match(src, /buzz-huddle-share-stage/);
  assert.match(src, /data-huddle-share-expanded/);
  assert.match(src, /Expand shared screen/);
  assert.match(src, /Collapse shared screen/);
  assert.match(src, /shareExpanded/);
  assert.doesNotMatch(src, /requestFullscreen|webkitRequestFullscreen/);
});

test("HuddleBar removes dock-to-main / return-to-drawer affordance", () => {
  assert.doesNotMatch(src, /close_huddle_companion/);
  assert.doesNotMatch(src, /Return huddle to drawer/);
  assert.doesNotMatch(src, /handleReturnToDrawer/);
  // Open companion from main remains; room no longer docks back.
  assert.match(src, /Open huddle window/);
  assert.match(src, /mode === "main"/);
});

test("expand remounts drawer-height sync without min-height ratchet", () => {
  // Clearing the CSS var before measuring prevents min-h-(--buzz-huddle-drawer-height)
  // from holding the dock at the pre-expand (preview-in-bar) height.
  assert.match(src, /removeProperty\("--buzz-huddle-drawer-height"\)/);
  assert.match(src, /scrollHeight/);
  assert.match(
    src,
    /\[shareStageHost, shareExpanded, spotlightStream\]/,
  );
});

test("share stage sits above app-surface so expand is visible", () => {
  // .buzz-huddle-app-surface uses z-10; stage must be higher or the share
  // is painted behind the transcript and appears to vanish on expand.
  const stageBlock = css.match(
    /\.buzz-huddle-share-stage\s*\{[^}]+\}/,
  );
  assert.ok(stageBlock, "expected .buzz-huddle-share-stage rule");
  assert.match(stageBlock[0], /z-index:\s*15/);
  assert.match(stageBlock[0], /bottom:\s*var\(--buzz-huddle-drawer-height\)/);
});

test("HuddleBar passes republishing overlay while local share is reconnecting", () => {
  assert.match(src, /republishing=\{screenShare\.sharing && screenShare\.republishing\}/);
});

test("expanded share shows dock Chat button and keeps participant avatars", () => {
  assert.match(src, /HuddleDockChatControl/);
  assert.match(src, /huddle-dock-chat-button|HuddleDockChatControl/);
  assert.match(src, /visible=\{Boolean\(shareExpanded && spotlightStream\)\}/);
  // Avatars stay in the dock while expanded (room header is covered by stage).
  assert.match(src, /mode === "main" \|\| shareExpanded/);
  assert.match(src, /HuddleParticipantsControl/);
  assert.match(src, /setHuddleShareExpanded/);
});

test("dock puts react/captions/agent/share beside Leave so avatars get the left", () => {
  // Preferred layout: no centered middle column squeezing the avatar strip.
  assert.match(src, /grid-cols-\[minmax\(0,1fr\)_auto\]/);
  assert.doesNotMatch(
    src,
    /grid-cols-\[minmax\(0,1fr\)_auto_minmax\(0,1fr\)\]/,
  );
  assert.doesNotMatch(src, /justify-self-center/);
  assert.match(src, /justify-self-end/);
  // Avatar strip sits in the flexible left cluster.
  assert.match(src, /className="min-w-0 flex-1"/);
});

test("bar avatar strip avoids overflow-x-auto scrollbars", () => {
  // overflow-x-auto makes overflow-y compute to auto, which paints scrollbars
  // when speaking rings/glow extend past the strip height.
  assert.match(
    participantListSrc,
    /appearance === "room"[\s\S]*?overflow-x-auto[\s\S]*?: "h-12 gap-1 overflow-visible"/,
  );
});

test("HuddleBar passes remote LiveKit track into spotlight for attach", () => {
  assert.match(src, /spotlightVideoTrack/);
  assert.match(src, /videoTrack=\{spotlightVideoTrack\}/);
  assert.match(src, /remoteVideoTrack/);
});
