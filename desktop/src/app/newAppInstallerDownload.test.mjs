import assert from "node:assert/strict";
import test from "node:test";

import {
  dialogDescription,
  dialogTitle,
  downloadPercent,
  downloadProgressText,
  failureMessage,
  folderName,
  formatBytes,
  INSTALLER_DIALOG_CLOSED,
  liveAnnouncement,
  reduceInstallerDialog,
  revealLabel,
} from "./newAppInstallerDownload.ts";

const FILE = "Buzz_1.0.0_aarch64.dmg";
const MIB = 1024 * 1024;

function run(actions, state = INSTALLER_DIALOG_CLOSED) {
  return actions.reduce(reduceInstallerDialog, state);
}

const started = { type: "started" };
const named = { type: "event", event: { kind: "started", fileName: FILE } };
const progress = (receivedBytes, totalBytes) => ({
  type: "event",
  event: { kind: "progress", receivedBytes, totalBytes },
});
const saved = {
  type: "settled",
  outcome: { status: "saved", fileName: FILE, folder: "/Users/sam/Downloads" },
};
const cancelled = { type: "settled", outcome: { status: "cancelled" } };
const failed = (reason) => ({
  type: "settled",
  outcome: { status: "failed", reason },
});

test("a download runs from start through progress to finished", () => {
  let state = run([started]);
  assert.deepEqual(state, {
    status: "downloading",
    fileName: null,
    receivedBytes: 0,
    totalBytes: null,
    isCancelling: false,
  });

  state = run(
    [named, progress(0, 80 * MIB), progress(20 * MIB, 80 * MIB)],
    state,
  );
  assert.equal(state.fileName, FILE);
  assert.equal(state.receivedBytes, 20 * MIB);
  assert.equal(state.totalBytes, 80 * MIB);

  // A late, lower report never moves the bar back.
  state = run([progress(10 * MIB, 80 * MIB)], state);
  assert.equal(state.receivedBytes, 20 * MIB);

  state = run([saved], state);
  assert.deepEqual(state, {
    status: "finished",
    fileName: FILE,
    folder: "/Users/sam/Downloads",
    revealFailed: false,
  });
  assert.equal(
    run([{ type: "revealSettled", ok: false }], state).revealFailed,
    true,
  );
  assert.deepEqual(
    run([{ type: "dismissed" }], state),
    INSTALLER_DIALOG_CLOSED,
  );
});

test("a second start while one is running changes nothing", () => {
  const running = run([started, named, progress(5, 10)]);
  assert.equal(run([started], running), running);
});

test("a running download cannot be dismissed, only cancelled", () => {
  const running = run([started, named]);
  assert.equal(run([{ type: "dismissed" }], running), running);

  const cancelling = run([{ type: "cancelRequested" }], running);
  assert.equal(cancelling.isCancelling, true);
  assert.deepEqual(run([cancelled], cancelling), INSTALLER_DIALOG_CLOSED);
  // A failure that races the cancel is what the user asked for: it closes.
  assert.deepEqual(
    run([failed("network")], cancelling),
    INSTALLER_DIALOG_CLOSED,
  );
  // A download that finished before the cancel landed is still reported.
  assert.equal(run([saved], cancelling).status, "finished");
  // If the cancel request itself fails, the Cancel button comes back.
  assert.equal(run([{ type: "cancelFailed" }], cancelling).isCancelling, false);
});

test("a failure can be retried or dismissed", () => {
  const failure = run([started, named, failed("network")]);
  assert.deepEqual(failure, { status: "failed", reason: "network" });
  assert.equal(run([started], failure).status, "downloading");
  assert.deepEqual(
    run([{ type: "dismissed" }], failure),
    INSTALLER_DIALOG_CLOSED,
  );
});

test("events and results are ignored once the download is over", () => {
  for (const state of [
    INSTALLER_DIALOG_CLOSED,
    run([started, saved]),
    run([started, failed("storage")]),
  ]) {
    for (const action of [
      named,
      progress(1, 2),
      saved,
      cancelled,
      failed("network"),
      { type: "cancelRequested" },
      { type: "cancelFailed" },
    ]) {
      assert.equal(reduceInstallerDialog(state, action), state);
    }
  }
});

test("percent is whole, clamped, and absent without a total", () => {
  assert.equal(downloadPercent(0, 80), 0);
  assert.equal(downloadPercent(79, 80), 98);
  assert.equal(downloadPercent(80, 80), 100);
  assert.equal(downloadPercent(90, 80), 100);
  assert.equal(downloadPercent(10, null), null);
  assert.equal(downloadPercent(10, 0), null);
});

test("bytes and progress read naturally", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1023), "1023 B");
  assert.equal(formatBytes(1536), "2 KB");
  assert.equal(formatBytes(79 * MIB), "79.0 MB");
  assert.equal(formatBytes(1.5 * 1024 * MIB), "1.50 GB");
  assert.equal(
    downloadProgressText(39.5 * MIB, 79 * MIB),
    "39.5 MB of 79.0 MB (50%)",
  );
  assert.equal(downloadProgressText(12.3 * MIB, null), "12.3 MB downloaded");
});

test("the folder name is the last path segment on any platform", () => {
  assert.equal(folderName("/Users/sam/Downloads"), "Downloads");
  assert.equal(folderName("/home/sam/Downloads/"), "Downloads");
  assert.equal(folderName("C:\\Users\\sam\\Downloads"), "Downloads");
  assert.equal(folderName(""), "");
});

test("the reveal button names each platform's file manager", () => {
  assert.equal(revealLabel("macos"), "Show in Finder");
  assert.equal(revealLabel("windows"), "Show in File Explorer");
  assert.equal(revealLabel("linux"), "Show in folder");
});

test("every failure reason has its own plain-words message", () => {
  const reasons = [
    "busy",
    "rejected",
    "network",
    "too-large",
    "storage",
    "protection",
    "unknown",
  ];
  const messages = reasons.map(failureMessage);
  assert.equal(new Set(messages).size, reasons.length);
  for (const message of messages) {
    assert.match(message, /^[A-Z].*\.$/);
  }
});

test("titles and descriptions follow the state", () => {
  const starting = run([started]);
  assert.equal(dialogTitle(starting), "Downloading the new Buzz app");
  assert.equal(dialogDescription(starting), "Starting the download…");

  const running = run([named], starting);
  assert.equal(
    dialogDescription(running),
    `Saving ${FILE} to your Downloads folder.`,
  );
  assert.equal(
    dialogDescription(run([{ type: "cancelRequested" }], running)),
    "Cancelling the download…",
  );

  const finished = run([saved], running);
  assert.equal(dialogTitle(finished), "The new Buzz app is downloaded");
  assert.equal(
    dialogDescription(finished),
    `${FILE} is in your Downloads folder. Open it when you're ready to install.`,
  );

  const failure = run([failed("network")], running);
  assert.equal(dialogTitle(failure), "The download didn't finish");
  assert.match(dialogDescription(failure), /internet connection/);
  assert.match(dialogDescription(failure), /buzz\.xyz\.$/);
});

test("a screen reader hears milestones, not every progress tick", () => {
  let state = run([started]);
  const heard = [liveAnnouncement(state)];
  const actions = [named, progress(0, 1000)];
  // A report for every 1% of the file.
  for (let received = 10; received <= 1000; received += 10) {
    actions.push(progress(received, 1000));
  }
  actions.push(saved);
  for (const action of actions) {
    state = reduceInstallerDialog(state, action);
    const announcement = liveAnnouncement(state);
    if (announcement !== heard.at(-1)) {
      heard.push(announcement);
    }
  }
  assert.deepEqual(heard, [
    "Starting the download.",
    `Downloading ${FILE}.`,
    `Downloading ${FILE}, 25% done.`,
    `Downloading ${FILE}, 50% done.`,
    `Downloading ${FILE}, 75% done.`,
    `Downloading ${FILE}, 100% done.`,
    `Download finished. ${FILE} is in your Downloads folder.`,
  ]);

  // Without a total there is nothing to count: one announcement while it runs.
  state = run([started, named]);
  const unsized = new Set([liveAnnouncement(state)]);
  for (let received = 10; received <= 1000; received += 10) {
    state = reduceInstallerDialog(state, progress(received, null));
    unsized.add(liveAnnouncement(state));
  }
  assert.deepEqual([...unsized], [`Downloading ${FILE}.`]);

  assert.equal(
    liveAnnouncement(run([started, named, { type: "cancelRequested" }])),
    "Cancelling the download.",
  );
  assert.match(
    liveAnnouncement(run([started, failed("storage")])),
    /^The download didn't finish\. /,
  );
  assert.equal(liveAnnouncement(INSTALLER_DIALOG_CLOSED), "");
});
