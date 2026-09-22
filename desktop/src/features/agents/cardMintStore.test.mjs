import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

// cardMintStore drives the non-blocking mint flow: the dialog dispatches a
// job and closes; the composer chip, completion toast, and viewer all read
// this store. These tests exercise the job lifecycle with an injected mintFn
// (never the real Tauri command).
//
// The store imports sonner (toast); calling toast outside a mounted <Toaster>
// only queues, so no DOM is required.

import {
  dismissCardMintJob,
  getCardGalleryOpen,
  getCardMintJobs,
  getCardViewerState,
  closeCardViewer,
  openCardViewer,
  resetCardMintStore,
  runCardMintJob,
  setCardGalleryOpen,
  subscribeCardMintStore,
  viewMintedCardJob,
} from "./cardMintStore.ts";

const CARD = {
  cardPngBase64: "aGVsbG8=",
  fileName: "eva.agent.png",
  designerNotes: "notes",
  locked: false,
  memoryLevel: "none",
};

const INPUT = { agentId: "agent-1", agentName: "Eva" };

describe("cardMintStore", () => {
  beforeEach(() => {
    resetCardMintStore();
  });

  it("forwards the mint input — including memoryLevel — to mintFn", async () => {
    const seen = [];
    await runCardMintJob(
      { ...INPUT, styleNotes: "stormy", lock: true, memoryLevel: "core" },
      (...args) => {
        seen.push(args);
        return Promise.resolve({ ...CARD, memoryLevel: "core" });
      },
    );
    assert.deepEqual(seen, [["agent-1", "stormy", true, "core"]]);

    // Omitted memoryLevel stays undefined so Rust applies its "none" default.
    await runCardMintJob(INPUT, (...args) => {
      seen.push(args);
      return Promise.resolve(CARD);
    });
    assert.deepEqual(seen[1], ["agent-1", undefined, undefined, undefined]);
  });

  it("tracks a successful mint through minting → done", async () => {
    let resolveMint;
    const pending = new Promise((resolve) => {
      resolveMint = resolve;
    });
    const run = runCardMintJob(INPUT, () => pending);

    let jobs = getCardMintJobs();
    assert.equal(jobs.length, 1);
    assert.equal(jobs[0].phase, "minting");
    assert.equal(jobs[0].input.agentName, "Eva");
    assert.equal(jobs[0].card, null);

    resolveMint(CARD);
    await run;

    jobs = getCardMintJobs();
    assert.equal(jobs.length, 1);
    assert.equal(jobs[0].phase, "done");
    assert.deepEqual(jobs[0].card, CARD);
    assert.equal(jobs[0].error, null);
  });

  it("records the error message on a failed mint", async () => {
    await runCardMintJob(INPUT, () => Promise.reject(new Error("boom")));
    const jobs = getCardMintJobs();
    assert.equal(jobs.length, 1);
    assert.equal(jobs[0].phase, "error");
    assert.equal(jobs[0].error, "boom");
    assert.equal(jobs[0].card, null);
  });

  it("strips the NO_OPENAI_KEY wire prefix from mint errors", async () => {
    await runCardMintJob(INPUT, () =>
      Promise.reject(new Error("NO_OPENAI_KEY: No OPENAI_API_KEY found.")),
    );
    assert.equal(getCardMintJobs()[0].error, "No OPENAI_API_KEY found.");
  });

  it("replaces a 401 HTTP error with an actionable update-key message", async () => {
    await runCardMintJob(INPUT, () =>
      Promise.reject(
        new Error(
          "Card mint failed (HTTP 401 Unauthorized): Incorrect API key provided: sk-proj-***",
        ),
      ),
    );
    const { error } = getCardMintJobs()[0];
    assert.ok(
      error?.includes("invalid or expired"),
      `expected 'invalid or expired' in: ${error}`,
    );
    assert.ok(
      error?.includes("Update API key"),
      `expected 'Update API key' in: ${error}`,
    );
  });

  it("replaces an 'Incorrect API key' error without an HTTP status code", async () => {
    await runCardMintJob(INPUT, () =>
      Promise.reject(new Error("Incorrect API key provided: sk-proj-***")),
    );
    const { error } = getCardMintJobs()[0];
    assert.ok(
      error?.includes("invalid or expired"),
      `expected 'invalid or expired' in: ${error}`,
    );
  });

  it("does not apply the 401 branch to generic non-auth errors", async () => {
    await runCardMintJob(INPUT, () =>
      Promise.reject(new Error("Connection timeout")),
    );
    assert.equal(getCardMintJobs()[0].error, "Connection timeout");
  });

  it("does not rewrite avatar fetch 401 as an API key error", async () => {
    // Avatar fetch failures have a different error prefix — rewriting them
    // would send the user down a path that cannot fix the avatar failure.
    const avatarError = "Avatar fetch failed: HTTP 401 Unauthorized";
    await runCardMintJob(INPUT, () => Promise.reject(new Error(avatarError)));
    assert.equal(
      getCardMintJobs()[0].error,
      avatarError,
      "avatar 401 must pass through unchanged",
    );
  });

  it("viewMintedCardJob moves a done job into the viewer and clears the chip", async () => {
    await runCardMintJob(INPUT, () => Promise.resolve(CARD));
    const jobId = getCardMintJobs()[0].jobId;

    viewMintedCardJob(jobId);

    assert.equal(getCardMintJobs().length, 0);
    const viewer = getCardViewerState();
    assert.ok(viewer);
    assert.equal(viewer.agentName, "Eva");
    assert.deepEqual(viewer.card, CARD);
    // Fresh mints keep their input so the viewer can reroll.
    assert.deepEqual(viewer.remint, INPUT);
  });

  it("viewMintedCardJob ignores jobs that are still minting", () => {
    let resolveMint;
    void runCardMintJob(
      INPUT,
      () => new Promise((resolve) => (resolveMint = resolve)),
    );
    const jobId = getCardMintJobs()[0].jobId;

    viewMintedCardJob(jobId);

    assert.equal(getCardMintJobs().length, 1);
    assert.equal(getCardViewerState(), null);
    resolveMint(CARD); // avoid a dangling promise
  });

  it("dismissCardMintJob removes only the named job", async () => {
    await runCardMintJob(INPUT, () => Promise.reject(new Error("a")));
    await runCardMintJob({ agentId: "agent-2", agentName: "Wren" }, () =>
      Promise.reject(new Error("b")),
    );
    const [first, second] = getCardMintJobs();

    dismissCardMintJob(first.jobId);

    const jobs = getCardMintJobs();
    assert.equal(jobs.length, 1);
    assert.equal(jobs[0].jobId, second.jobId);
  });

  it("openCardViewer/closeCardViewer manage archive views without remint", () => {
    openCardViewer({ card: CARD, agentName: "Eva", remint: null });
    assert.equal(getCardViewerState()?.remint, null);
    closeCardViewer();
    assert.equal(getCardViewerState(), null);
  });

  it("gallery open flag toggles and notifies subscribers", () => {
    let notified = 0;
    const unsubscribe = subscribeCardMintStore(() => {
      notified += 1;
    });
    setCardGalleryOpen(true);
    assert.equal(getCardGalleryOpen(), true);
    setCardGalleryOpen(true); // no-op must not notify
    setCardGalleryOpen(false);
    assert.equal(getCardGalleryOpen(), false);
    assert.equal(notified, 2);
    unsubscribe();
  });

  it("concurrent jobs keep distinct snapshots (referential updates)", async () => {
    const before = getCardMintJobs();
    await runCardMintJob(INPUT, () => Promise.resolve(CARD));
    const after = getCardMintJobs();
    assert.notEqual(before, after, "snapshot identity must change on update");
  });
});
