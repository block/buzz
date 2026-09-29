import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const src = fs.readFileSync(
  path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "useHuddleScreenShare.ts",
  ),
  "utf8",
);

test("startShare sets local preview before mint/connect", () => {
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  const acquireIdx = body.indexOf("await acquireDisplayMedia()");
  const previewIdx = body.indexOf("setLocalPreviewStream(acquired)");
  const mintIdx = body.indexOf("mintScreenShareToken");
  assert.ok(acquireIdx >= 0, "acquires display media");
  assert.ok(previewIdx > acquireIdx, "preview after acquire");
  assert.ok(mintIdx > previewIdx, "mint after preview");
  assert.match(body, /setSharing\(true\)/);
});

test("startShare clears preview only on unavailable or pre-hold failure", () => {
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  // True unavailable still tears down capture.
  assert.match(body, /unavailable/);
  assert.match(body, /stopMediaStreamTracks\(acquired\)/);
  // After hold, mint/connect failure enters republish — must not clear preview.
  assert.match(body, /republish loop/);
  assert.match(body, /scheduleRepublishRef/);
});

test("PC connect failure does not set available false", () => {
  // Only relay unavailable may hide Share. Catch paths must not demote
  // available after a mint that proved LiveKit is configured.
  const catchBlocks = [...src.matchAll(/\} catch \(e\) \{([\s\S]*?)\n      \}/g)].map(
    (m) => m[1],
  );
  assert.ok(catchBlocks.length >= 2, "expected subscriber + startShare catches");
  for (const block of catchBlocks) {
    assert.doesNotMatch(
      block,
      /setAvailable\(false\)/,
      "connect/share catch must not hide Share",
    );
  }
  assert.match(src, /setAvailable\(false\)/);
  assert.match(src, /unavailable/);
});

test("connect generation invalidates stale subscriber after startShare", () => {
  assert.match(src, /connectGenRef/);
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  assert.match(body, /connectGenRef\.current \+= 1/);
  assert.match(src, /gen !== connectGenRef\.current/);
});

test("subscriber effect does not reconnect when parentChannelId changes", () => {
  const effect = src.slice(
    src.indexOf("React.useEffect(() => {"),
    src.indexOf("const startShare"),
  );
  assert.match(
    effect,
    /\[active, channelId, sessionCallbacks, teardown\]/,
  );
  assert.doesNotMatch(
    effect,
    /\[active, channelId, parentChannelId/,
  );
  assert.match(effect, /parentRef\.current/);
});

test("subscriber skips installing a session when startShare already owns one", () => {
  assert.match(src, /if \(sessionRef\.current\) \{\s*return;/);
});

test("benign AbortError does not setError in startShare or subscriber", () => {
  assert.match(src, /isBenignScreenShareAbort/);
  assert.match(src, /shouldSuppressScreenShareError/);
  const start = src.indexOf("const startShare");
  const startBody = src.slice(start, src.indexOf("const stopShare", start));
  // startShare catch must skip setError when abort/PC blip is suppressed
  assert.match(startBody, /shouldSuppressScreenShareError\(e\)/);
  const abortIdx = startBody.indexOf("shouldSuppressScreenShareError(e)");
  const setErrorIdx = startBody.indexOf("setError(e instanceof Error");
  assert.ok(abortIdx >= 0 && setErrorIdx > abortIdx, "suppress guard before setError");

  const effect = src.slice(
    src.indexOf("React.useEffect(() => {"),
    src.indexOf("const startShare"),
  );
  assert.match(effect, /shouldSuppressScreenShareError\(e\)/);
});

test("transient PC connection errors are suppressed (no red banner)", () => {
  assert.match(src, /isTransientPcConnectionError/);
  assert.match(src, /could not establish pc connection/);
  assert.match(src, /shouldSuppressScreenShareError/);
});

test("successful publish clears error", () => {
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  assert.match(body, /await session\.startShare\(acquired\)/);
  const shareIdx = body.indexOf("await session.startShare(acquired)");
  const clearIdx = body.indexOf("setError(null)", shareIdx);
  assert.ok(clearIdx > shareIdx, "clears error after successful publish");
});

test("stopShare restores Share availability and clears sharing state", () => {
  const stop = src.indexOf("const stopShare");
  const body = src.slice(stop, src.indexOf("const shareBlocked", stop));
  assert.match(body, /finally/);
  assert.match(body, /setSharing\(false\)/);
  assert.match(body, /setLocalPreviewStream\(null\)/);
  assert.match(
    body,
    /setAvailable\(\(prev\) => \(prev === false \? false : true\)\)/,
  );
});

test("permanent disconnect schedules soft resubscribe without setError", () => {
  assert.match(src, /softResubscribeRef/);
  assert.match(src, /onDisconnected:/);
  assert.match(src, /screen-share soft-resubscribe/);
});

test("exposes republishing and schedules publish reconnect loop", () => {
  assert.match(src, /republishing/);
  assert.match(src, /runRepublishLoop/);
  assert.match(src, /onPublishInterrupted/);
  assert.match(src, /scheduleRepublishRef/);
  assert.match(src, /REPUBLISH_BACKOFF_MS/);
  assert.match(src, /intent: "publish"/);
  // Soft resubscribe must not clobber an active local share.
  assert.match(src, /wantShareRef\.current \|\| heldStreamRef\.current/);
});

test("startShare keeps preview and sharing on connect failure after acquire", () => {
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  assert.match(body, /heldStreamRef\.current = acquired/);
  assert.match(body, /wantShareRef\.current = true/);
  // Catch path after hold schedules republish instead of clearing sharing.
  const catchIdx = body.lastIndexOf("} catch (e) {");
  const catchBody = body.slice(catchIdx);
  assert.match(catchBody, /scheduleRepublishRef/);
  assert.match(catchBody, /setSharing\(true\)/);
  assert.doesNotMatch(
    catchBody.slice(catchBody.indexOf("heldStreamRef.current")),
    /setLocalPreviewStream\(null\);\s*setSharing\(false\);\s*return;\s*\}\s*setError/,
  );
});

test("stopShare cancels republish generation", () => {
  const stop = src.indexOf("const stopShare");
  const body = src.slice(stop, src.indexOf("const shareBlocked", stop));
  assert.match(body, /republishGenRef\.current \+= 1/);
  assert.match(body, /wantShareRef\.current = false/);
  assert.match(body, /setRepublishing\(false\)/);
});
