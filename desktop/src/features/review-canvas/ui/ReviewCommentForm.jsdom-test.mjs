/**
 * Behavior of the Review Canvas comment form, rendered for real.
 *
 * Contract: the recovery path of a signed comment never strands keyboard
 * focus, and a locked draft is always offered as "Retry".
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { ReviewCommentForm } from "./ReviewCommentForm.tsx";

const BLOCK = {
  id: "checkout.primary-action",
  title: "Primary checkout action",
  sourceRef: null,
};

const BASE_PROPS = {
  block: BLOCK,
  value: "Move this above the order summary.",
  phase: { kind: "idle" },
  locked: false,
  feedbackPublished: false,
  publishState: null,
  canDiscard: false,
  blockedReason: null,
  onChange: () => {},
  onSubmit: () => {},
  onCancel: () => {},
  onDiscard: () => {},
};

const mounted = [];

function mount() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const view = {
    container,
    render: (element) =>
      act(async () => {
        root.render(element);
      }),
    unmount: () =>
      act(async () => {
        root.unmount();
        container.remove();
      }),
  };
  mounted.push(view);
  return view;
}

const form = (props = {}) =>
  React.createElement(ReviewCommentForm, { ...BASE_PROPS, ...props });

const submitButton = (view) =>
  view.container.querySelector('button[type="submit"]');
const textarea = (view) => view.container.querySelector("textarea");
const buttonNamed = (view, name) =>
  [...view.container.querySelectorAll("button")].find(
    (button) => button.textContent === name,
  );

function click(element) {
  return act(async () => {
    element.click();
  });
}

afterEach(async () => {
  while (mounted.length > 0) await mounted.pop().unmount();
});

test("a locked draft reads Retry in every phase, so a reopened signed comment is never offered as a new send", async () => {
  const view = mount();
  // Reopening a block resets the phase to idle while the draft stays signed.
  await view.render(form({ locked: true }));
  assert.equal(submitButton(view).textContent, "Retry");
  assert.equal(textarea(view).readOnly, true);

  await view.render(form({ locked: true, feedbackPublished: true }));
  assert.equal(submitButton(view).textContent, "Retry");

  await view.render(form({ locked: true, phase: { kind: "sending" } }));
  assert.equal(submitButton(view).textContent, "Sending…");

  // An unsigned comment is a plain send until something fails.
  await view.render(form());
  assert.equal(submitButton(view).textContent, "Send feedback");
  await view.render(form({ phase: { kind: "error", message: "nope" } }));
  assert.equal(submitButton(view).textContent, "Retry");
});

test("Discard hands focus to the text field it re-opens for editing", async () => {
  function SignedDraft() {
    const [locked, setLocked] = React.useState(true);
    return form({
      locked,
      canDiscard: locked,
      onDiscard: () => setLocked(false),
    });
  }
  const view = mount();
  await view.render(React.createElement(SignedDraft));
  const discard = buttonNamed(view, "Discard");
  discard.focus();
  assert.equal(document.activeElement, discard);

  await click(discard);

  assert.equal(buttonNamed(view, "Discard"), undefined);
  assert.equal(document.activeElement, textarea(view));
  assert.equal(textarea(view).readOnly, false);
  assert.equal(submitButton(view).textContent, "Send feedback");
});

test("Send stays focused and focusable while the send is in flight, and cannot be double-activated", async () => {
  let sends = 0;
  const onSubmit = () => {
    sends += 1;
  };
  const view = mount();
  await view.render(form({ onSubmit }));
  const send = submitButton(view);
  send.focus();

  await click(send);
  assert.equal(sends, 1);

  // The in-flight state must not use the `disabled` attribute: the browser
  // would move focus off a focused button that becomes disabled.
  await view.render(form({ onSubmit, phase: { kind: "sending" } }));
  assert.equal(submitButton(view), send, "the same button element persists");
  assert.equal(document.activeElement, send);
  assert.equal(send.disabled, false);
  assert.equal(send.getAttribute("aria-disabled"), "true");
  await click(send);
  assert.equal(sends, 1, "an in-flight send ignores activation");

  // On failure it is the focused, re-enabled Retry.
  await view.render(
    form({ onSubmit, locked: true, phase: { kind: "error", message: "x" } }),
  );
  assert.equal(document.activeElement, send);
  assert.equal(send.hasAttribute("aria-disabled"), false);
  assert.equal(send.textContent, "Retry");
});

test("an empty or paused comment cannot send, but its button stays reachable and says why", async () => {
  let sends = 0;
  const onSubmit = () => {
    sends += 1;
  };
  const view = mount();

  await view.render(form({ onSubmit, value: "   " }));
  assert.equal(submitButton(view).getAttribute("aria-disabled"), "true");
  assert.equal(submitButton(view).disabled, false);
  await click(submitButton(view));
  assert.equal(sends, 0);

  const reason = "Checking that this is still the latest revision…";
  await view.render(form({ onSubmit, blockedReason: reason }));
  const send = submitButton(view);
  assert.equal(send.getAttribute("aria-disabled"), "true");
  assert.equal(send.disabled, false);
  const reasonId = send.getAttribute("aria-describedby");
  assert.equal(document.getElementById(reasonId).textContent, reason);
  await click(send);
  assert.equal(sends, 0);

  await view.render(form({ onSubmit }));
  assert.equal(submitButton(view).hasAttribute("aria-disabled"), false);
  await click(submitButton(view));
  assert.equal(sends, 1);
});

test("Discard ignores activation while a send is in flight but keeps focus", async () => {
  let discards = 0;
  const onDiscard = () => {
    discards += 1;
  };
  const view = mount();
  await view.render(form({ locked: true, canDiscard: true, onDiscard }));
  const discard = buttonNamed(view, "Discard");
  discard.focus();

  await view.render(
    form({
      locked: true,
      canDiscard: true,
      onDiscard,
      phase: { kind: "sending" },
    }),
  );
  assert.equal(buttonNamed(view, "Discard"), discard);
  assert.equal(discard.disabled, false);
  assert.equal(discard.getAttribute("aria-disabled"), "true");
  assert.equal(document.activeElement, discard);
  await click(discard);
  assert.equal(discards, 0);
});

test("Discard is offered only when the relay provably holds nothing: never while ambiguous or published", async () => {
  const view = mount();
  for (const [publishState, feedbackPublished, canDiscard] of [
    ["never_attempted", false, true],
    ["rejected", false, true],
    ["ambiguous", false, false],
    ["accepted", true, false],
  ]) {
    await view.render(
      form({ locked: true, publishState, feedbackPublished, canDiscard }),
    );
    assert.equal(
      buttonNamed(view, "Discard") !== undefined,
      canDiscard,
      publishState,
    );
    assert.equal(submitButton(view).textContent, "Retry", publishState);
  }
});

test("a locked comment says what Retry will do for each thing known about the relay", async () => {
  const view = mount();
  const hint = () => view.container.querySelector("p[id]")?.textContent ?? "";
  await view.render(form({ locked: true, publishState: "ambiguous" }));
  assert.match(hint(), /has not confirmed it/);
  assert.match(hint(), /cannot be discarded/);
  await view.render(
    form({ locked: true, publishState: "rejected", canDiscard: true }),
  );
  assert.match(hint(), /refused this comment/);
  await view.render(
    form({ locked: true, publishState: "accepted", feedbackPublished: true }),
  );
  assert.match(hint(), /notify the agent/);
});

/** Ctrl+Enter (or ⌘+Enter) in the comment field; returns the keydown event. */
async function pressInTextarea(view, init) {
  const event = new window.KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
    ...init,
  });
  await act(async () => {
    textarea(view).dispatchEvent(event);
  });
  return event;
}

/** A submit that does not come from the button: implicit or programmatic. */
function submitForm(view) {
  return act(async () => {
    view.container
      .querySelector("form")
      .dispatchEvent(
        new window.Event("submit", { bubbles: true, cancelable: true }),
      );
  });
}

test("keyboard and form submission obey exactly the gate the Send button shows", async () => {
  let sends = 0;
  const onSubmit = () => {
    sends += 1;
  };
  const view = mount();
  const blockers = [
    ["a head check or newer revision", { blockedReason: "paused" }],
    ["an empty comment", { value: "  " }],
    ["a send in flight", { phase: { kind: "sending" } }],
    [
      "a discard in flight",
      { locked: true, canDiscard: true, phase: { kind: "discarding" } },
    ],
  ];
  for (const [name, props] of blockers) {
    await view.render(form({ onSubmit, ...props }));
    assert.equal(
      submitButton(view).getAttribute("aria-disabled"),
      "true",
      `${name}: the button reads unavailable`,
    );
    await pressInTextarea(view, { ctrlKey: true });
    await pressInTextarea(view, { metaKey: true });
    await submitForm(view);
    assert.equal(sends, 0, `${name}: no path submitted`);
  }

  // With nothing blocking, all three paths go through.
  await view.render(form({ onSubmit }));
  assert.equal(submitButton(view).hasAttribute("aria-disabled"), false);
  const shortcut = await pressInTextarea(view, { ctrlKey: true });
  assert.equal(shortcut.defaultPrevented, true);
  await pressInTextarea(view, { metaKey: true });
  await submitForm(view);
  assert.equal(sends, 3);

  // Enter alone is a newline, never a send.
  await pressInTextarea(view, {});
  assert.equal(sends, 3);
});

test("discarding is its own phase: Send and Retry stay unavailable under their own label, and Discard is guarded and keeps focus", async () => {
  let discards = 0;
  let sends = 0;
  const props = {
    locked: true,
    canDiscard: true,
    publishState: "rejected",
    onDiscard: () => {
      discards += 1;
    },
    onSubmit: () => {
      sends += 1;
    },
  };
  const view = mount();
  await view.render(form(props));
  // Cancel, Discard, Send/Retry.
  const [, discard, send] = view.container.querySelectorAll("button");
  assert.equal(send, submitButton(view));
  const idleDiscardLabel = discard.textContent;
  const idleSendLabel = send.textContent;
  discard.focus();

  await view.render(form({ ...props, phase: { kind: "discarding" } }));

  // The same elements persist, so focus is not lost to the page.
  assert.deepEqual([...view.container.querySelectorAll("button")].slice(1), [
    discard,
    send,
  ]);
  assert.equal(document.activeElement, discard);
  assert.equal(discard.disabled, false);
  assert.equal(discard.getAttribute("aria-disabled"), "true");
  assert.notEqual(discard.textContent, idleDiscardLabel, "progress is shown");

  // Send/Retry is unavailable, and neither claims to be sending nor flips to a
  // different action: it keeps the label it had before the discard.
  assert.equal(send.disabled, false);
  assert.equal(send.getAttribute("aria-disabled"), "true");
  assert.equal(send.textContent, idleSendLabel);
  await view.render(form({ ...props, phase: { kind: "sending" } }));
  assert.notEqual(
    send.textContent,
    idleSendLabel,
    "sending reads differently from discarding",
  );
  await view.render(form({ ...props, phase: { kind: "discarding" } }));

  // The comment is held read-only for the duration.
  assert.equal(textarea(view).readOnly, true);

  await click(discard);
  await click(send);
  assert.equal(discards, 0, "a discard in flight ignores another");
  assert.equal(sends, 0);

  // Settled (kept): Discard is offered again under its plain label, and works.
  await view.render(form(props));
  assert.equal(discard.textContent, idleDiscardLabel);
  assert.equal(send.hasAttribute("aria-disabled"), false);
  await click(discard);
  assert.equal(discards, 1);
});
