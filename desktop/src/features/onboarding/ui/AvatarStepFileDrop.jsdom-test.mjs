import assert from "node:assert/strict";
// jsdom's File has no arrayBuffer(), which the avatar upload reads.
import { File } from "node:buffer";
import fs from "node:fs";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, render, waitFor } from "@testing-library/react";

import { installFileDropNavigationGuard } from "../../../shared/lib/fileDropNavigationGuard.ts";
import { AvatarStep } from "./AvatarStep.tsx";

const UPLOADED_URL = "https://media.example/avatar.png";
const invokeCalls = [];
const tauriMock = {
  async invoke(command, args) {
    invokeCalls.push({ command, args });
    if (command === "upload_media_bytes") {
      return { url: UPLOADED_URL, type: "image/png", sha256: "a", size: 6 };
    }
    return null;
  },
  transformCallback() {
    return 0;
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
window.__TAURI_INTERNALS__ = tauriMock;
globalThis.ResizeObserver ??= class {
  observe() {}
  disconnect() {}
};
window.matchMedia ??= () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
});
URL.createObjectURL = () => "blob:local-preview";
URL.revokeObjectURL = () => {};

afterEach(() => {
  cleanup();
  invokeCalls.length = 0;
});

function dataTransferFor(files, types = ["Files"]) {
  return {
    dropEffect: "none",
    files,
    items: files.map((file) => ({ kind: "file", type: file.type })),
    types,
  };
}

function dispatchDrag(target, type, dataTransfer) {
  const event = new window.Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

function renderAvatarStep() {
  const avatarUrls = [];
  const view = render(
    React.createElement(AvatarStep, {
      actions: {
        advanceWithoutSaving() {},
        back() {},
        onUploadingChange() {},
        skipForNow() {},
        submit() {},
        updateAvatarUrl(url) {
          avatarUrls.push(url);
        },
      },
      direction: "forward",
      state: {
        avatar: { draftUrl: "" },
        isSaving: false,
        isUploadingAvatar: false,
        name: { draftValue: "Robin", savedValue: "" },
        saveRecovery: {
          canAdvanceWithoutSaving: false,
          canSkipForNow: false,
          errorMessage: null,
        },
      },
    }),
  );
  return { avatarUrls, view };
}

test("the app installs the file drop guard before rendering any screen", () => {
  const main = fs.readFileSync(new URL("../../../main.tsx", import.meta.url), {
    encoding: "utf8",
  });
  const bootstrap = main.slice(main.indexOf("async function bootstrap()"));
  const guardAt = bootstrap.indexOf("installFileDropNavigationGuard();");
  assert.ok(guardAt >= 0, "bootstrap must install the file drop guard");
  assert.ok(guardAt < bootstrap.indexOf("renderApp();"));
});

test("a file dropped on onboarding never navigates, and text drags stay native", () => {
  const removeGuard = installFileDropNavigationGuard();
  try {
    const { view } = renderAvatarStep();
    const heading = view.getByRole("heading", {
      name: "Next, add a display image",
    });
    const pdf = new File(["%PDF"], "notes.pdf", {
      type: "application/pdf",
    });

    assert.equal(
      dispatchDrag(heading, "dragover", dataTransferFor([pdf]))
        .defaultPrevented,
      true,
    );
    assert.equal(
      dispatchDrag(heading, "drop", dataTransferFor([pdf])).defaultPrevented,
      true,
    );
    assert.equal(
      dispatchDrag(heading, "drop", dataTransferFor([], ["text/plain"]))
        .defaultPrevented,
      false,
    );
    assert.equal(
      invokeCalls.some(({ command }) => command === "upload_media_bytes"),
      false,
    );
  } finally {
    removeGuard();
  }

  const { view } = renderAvatarStep();
  const pdf = new File(["%PDF"], "notes.pdf", { type: "application/pdf" });
  assert.equal(
    dispatchDrag(
      view.getAllByRole("heading")[0],
      "drop",
      dataTransferFor([pdf]),
    ).defaultPrevented,
    false,
    "without the guard the webview would navigate to the file",
  );
});

test("an image dropped on the avatar preview uploads it as the avatar", async () => {
  const { avatarUrls, view } = renderAvatarStep();
  const dropTarget = view.getByTestId("onboarding-avatar-drop-target");
  const image = new File(["avatar"], "avatar.png", {
    type: "image/png",
  });

  dispatchDrag(dropTarget, "dragenter", dataTransferFor([image]));
  const dragOver = dispatchDrag(
    dropTarget,
    "dragover",
    dataTransferFor([image]),
  );
  assert.equal(dragOver.defaultPrevented, true);
  assert.equal(dropTarget.getAttribute("data-dragging"), "true");
  assert.equal(
    view.getByTestId("onboarding-avatar-upload").getAttribute("data-dragging"),
    "true",
  );

  const drop = dispatchDrag(dropTarget, "drop", dataTransferFor([image]));
  assert.equal(drop.defaultPrevented, true);
  assert.equal(dropTarget.getAttribute("data-dragging"), null);

  await waitFor(() => assert.deepEqual(avatarUrls, [UPLOADED_URL]));
  const uploads = invokeCalls.filter(
    ({ command }) => command === "upload_media_bytes",
  );
  assert.equal(uploads.length, 1);
  assert.deepEqual(uploads[0].args.data, [
    ...new TextEncoder().encode("avatar"),
  ]);
});
