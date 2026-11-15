/**
 * Exercises the real hook against a fake backend: the download belongs to the
 * app, so it outlives the component that started it; mounting stops only a
 * download this page knows nothing about; and an earlier attempt's events
 * never reach the next attempt.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

const FILE = "Buzz_1.0.0_aarch64.dmg";
const URL = `https://github.com/block/buzz-app/releases/download/v1.0.0/${FILE}`;
const DOWNLOAD = "download_new_app_installer";
const CANCEL = "cancel_new_app_installer_download";
const REVEAL = "reveal_new_app_installer";

const calls = [];
/** One entry per `download_new_app_installer` call, in order. */
const downloads = [];
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === DOWNLOAD) {
      return new Promise((settle) => {
        downloads.push({
          emit: (event) => args.onEvent.onmessage(event),
          settle,
        });
      });
    }
    if (command === CANCEL || command === REVEAL) {
      return Promise.resolve(null);
    }
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { useNewAppInstallerDownload } = await import(
  "./useNewAppInstallerDownload.ts"
);

let hook = null;
function Probe() {
  hook = useNewAppInstallerDownload();
  return null;
}

let root = null;
let container = null;
async function mount() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(React.createElement(Probe));
  });
}
async function unmount() {
  await act(async () => root.unmount());
  container.remove();
  root = null;
  hook = null;
}
/** Runs `step`, then lets its promises and the resulting render settle. */
async function settled(step) {
  await act(async () => {
    step();
  });
}
const count = (command) => calls.filter((call) => call === command).length;

afterEach(async () => {
  if (root) {
    await unmount();
  }
});

test("a download outlives its component; mounting stops only a stranger", async () => {
  // Nothing is running in this page, so a download the backend still holds
  // was left by a page that was reloaded: it is stopped.
  await mount();
  assert.deepEqual(calls, [CANCEL]);
  assert.equal(hook.state.status, "closed");

  await settled(() => hook.start(URL));
  assert.equal(hook.state.status, "downloading");
  assert.equal(hook.state.url, URL);
  await settled(() => downloads[0].emit({ kind: "started", fileName: FILE }));

  // The banner's owner remounts on a community switch or an identity change.
  // That neither cancels the download nor loses its dialog.
  await unmount();
  assert.equal(count(CANCEL), 1);
  downloads[0].emit({ kind: "progress", receivedBytes: 40, totalBytes: 100 });
  await mount();
  assert.equal(count(CANCEL), 1, "a remount must not cancel its own download");
  assert.deepEqual(hook.state, {
    status: "downloading",
    url: URL,
    fileName: FILE,
    receivedBytes: 40,
    totalBytes: 100,
    isCancelling: false,
  });

  // It still progresses and finishes in the new component.
  await settled(() =>
    downloads[0].emit({ kind: "progress", receivedBytes: 90, totalBytes: 100 }),
  );
  assert.equal(hook.state.receivedBytes, 90);
  await settled(() =>
    downloads[0].settle({
      status: "saved",
      fileName: FILE,
      folder: "/Users/sam/Downloads",
    }),
  );
  assert.equal(hook.state.status, "finished");
  assert.equal(count(REVEAL), 1);

  await settled(() => hook.dismiss());
  assert.equal(hook.state.status, "closed");
  assert.equal(count(DOWNLOAD), 1);
});

test("an earlier attempt's events never reach the next attempt", async () => {
  await mount();
  const first = downloads.length;

  // Two clicks in one tick start one download.
  await settled(() => {
    hook.start(URL);
    hook.start(URL);
  });
  assert.equal(downloads.length, first + 1);
  await settled(() =>
    downloads[first].settle({ status: "failed", reason: "network" }),
  );
  assert.deepEqual(hook.state, {
    status: "failed",
    reason: "network",
    url: URL,
  });

  await settled(() => hook.start(hook.state.url));
  assert.equal(downloads.length, first + 2);
  const fresh = { ...hook.state };
  assert.equal(fresh.status, "downloading");
  assert.equal(fresh.receivedBytes, 0);

  // Channel messages of the failed attempt can still be in flight.
  await settled(() => {
    downloads[first].emit({ kind: "started", fileName: "stale.dmg" });
    downloads[first].emit({
      kind: "progress",
      receivedBytes: 99,
      totalBytes: 100,
    });
  });
  assert.deepEqual(hook.state, fresh);

  await settled(() =>
    downloads[first + 1].emit({
      kind: "progress",
      receivedBytes: 10,
      totalBytes: 100,
    }),
  );
  assert.equal(hook.state.receivedBytes, 10);

  // Cancelling asks the backend, and the dialog closes once it has stopped.
  const cancels = count(CANCEL);
  await settled(() => hook.cancel());
  assert.equal(count(CANCEL), cancels + 1);
  assert.equal(hook.state.isCancelling, true);
  await settled(() => downloads[first + 1].settle({ status: "cancelled" }));
  assert.equal(hook.state.status, "closed");
});
