import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

let act;
let cleanup;
let createElement;
let fireEvent;
let render;
let waitFor;
let ThemeProvider;
let TerminalSubstrate;
let reducedMotion = false;

// `pretendToBeVisual` is what gives jsdom requestAnimationFrame. The banner's
// animation loop needs it; without it the loop silently never runs and every
// paint assertion below reads as a rendering failure.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    HTMLCanvasElement: dom.window.HTMLCanvasElement,
    KeyboardEvent: dom.window.KeyboardEvent,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  // `navigator` is a getter-only global on Node, and the chord layer branches
  // on `isMacPlatform()`. Pin it so these assertions describe macOS behaviour
  // on every host instead of quietly inverting on a Linux runner.
  Object.defineProperty(dom.window.navigator, "platform", {
    configurable: true,
    value: "MacIntel",
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    get matches() {
      return reducedMotion;
    },
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.HTMLElement.prototype.animate = () => ({
    cancel() {},
    currentTime: 0,
    finished: new Promise(() => {}),
    play() {},
    playbackRate: 1,
    reverse() {},
  });
  ({ act, cleanup, fireEvent, render, waitFor } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider"));
  ({ TerminalSubstrate } = await import("./TerminalSubstrate.tsx"));
});

after(() => dom.window.close());
beforeEach(() => {
  cleanup?.();
  reducedMotion = false;
  paintLog.length = 0;
  dom.window.localStorage.clear();
  dom.window.HTMLCanvasElement.prototype.getBoundingClientRect = () => ({
    bottom: 782,
    height: 782,
    left: 0,
    right: 940.8,
    top: 0,
    width: 940.8,
    x: 0,
    y: 0,
    toJSON() {},
  });
  dom.window.HTMLCanvasElement.prototype.getContext = function () {
    return paintRecorder(this);
  };
});

// The default stub discards everything it is handed, so an assertion made
// against it cannot distinguish "repainted" from "drew nothing". This records
// every draw call against the canvas that received it, so the tests below are
// about commands actually issued and can exclude the banner canvas.
const paintLog = [];

function paintRecorder(canvas) {
  return {
    clearRect() {},
    fillRect(x, y, width, height) {
      paintLog.push({
        canvas,
        height,
        kind: "fill",
        style: this.fillStyle,
        width,
        x,
        y,
      });
    },
    fillStyle: "",
    fillText(text, x, y) {
      paintLog.push({ canvas, kind: "text", text, x, y });
    },
    font: "",
    restore() {},
    save() {},
    setTransform() {},
    textBaseline: "",
  };
}

/** Draws issued to the grid canvas only, newest paint pass included. */
function gridDraws(view) {
  const grid = view.container.querySelector(
    ".buzz-terminal-viewport > canvas:not(.buzz-terminal-welcome)",
  );
  return paintLog.filter((entry) => entry.canvas === grid);
}

function fixture(overrides = {}) {
  const calls = { input: [], scroll: [] };
  const props = {
    bracketedPaste: false,
    channelName: "terminal-test",
    focusReportingEnabled: false,
    onCloseSession() {},
    onInput(value) {
      calls.input.push(value);
    },
    onNewSession() {},
    onScroll(lines) {
      calls.scroll.push(lines);
    },
    onSelectSession() {},
    onTerminalFocusChange() {},
    sessions: [{ active: true, closing: false, id: "one", title: "SHELL" }],
    ...overrides,
  };
  const renderTree = (nextProps) =>
    createElement(
      ThemeProvider,
      null,
      createElement("div", {
        className: "buzz-huddle-app-surface",
        tabIndex: -1,
      }),
      createElement(TerminalSubstrate, nextProps),
    );
  const view = render(renderTree(props));
  return {
    calls,
    props,
    view,
    rerender(nextOverrides) {
      view.rerender(renderTree({ ...props, ...nextOverrides }));
    },
  };
}

async function ready(view) {
  await waitFor(() =>
    assert.ok(view.container.querySelector(".buzz-terminal-substrate")),
  );
}

function toggleChord(composing = false) {
  const init = {
    bubbles: true,
    code: "KeyJ",
    isComposing: composing,
    metaKey: true,
  };
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", init));
    window.dispatchEvent(new KeyboardEvent("keyup", init));
  });
}

test("mounted IME paths neither toggle nor emit preedit text", async () => {
  const { calls, view } = fixture();
  await ready(view);
  const substrate = view.container.querySelector(".buzz-terminal-substrate");
  toggleChord(true);
  assert.equal(substrate.dataset.terminalOwner, "buzz");

  toggleChord();
  await waitFor(() =>
    assert.equal(substrate.dataset.terminalOwner, "terminal"),
  );
  const input = view.getByLabelText("Terminal input");
  fireEvent.compositionStart(input);
  fireEvent.input(input, { target: { value: "k" } });
  fireEvent.input(input, { target: { value: "か" } });
  assert.deepEqual(calls.input, []);
  fireEvent.compositionEnd(input, { data: "か" });
  assert.deepEqual(calls.input, []);
  fireEvent.input(input, { target: { value: "か" } });
  assert.deepEqual(calls.input, ["か"]);
});

test("tab actions restore terminal input focus", async () => {
  const { view } = fixture();
  await ready(view);
  toggleChord();
  const input = view.getByLabelText("Terminal input");
  await waitFor(() => assert.equal(document.activeElement, input));

  const actions = [
    ["select", view.getByRole("tab")],
    ["close", view.getByLabelText("Close SHELL")],
    ["new", view.getByLabelText("New Buzz Term tab")],
  ];
  for (const [label, target] of actions) {
    target.focus();
    assert.equal(document.activeElement, target, `${label} takes focus`);
    fireEvent.click(target);
    assert.equal(
      document.activeElement,
      input,
      `${label} restores terminal focus`,
    );
  }
});

const EMPTY_FRAME = {
  cursor: { column: 0, line: 0, visible: false },
  full: false,
  rows: [],
  viewport: { columns: 112, generation: 1, screenLines: 46 },
};

const VISIBLE_FRAME = {
  ...EMPTY_FRAME,
  viewport: { ...EMPTY_FRAME.viewport, generation: 2 },
  rows: [
    {
      line: 0,
      spans: [
        {
          style: { fg: 0, bg: 0, flags: 0 },
          clusters: [{ column: 0, text: "$", width: 1 }],
        },
      ],
    },
  ],
};

async function expectWelcome(view, present) {
  // Compare a boolean, never the node: an AssertionError carrying a mounted
  // element inspects it to build its message, and `__reactFiber$` makes that
  // walk the whole fiber graph — a failure takes ~2min to report instead of ms.
  await waitFor(() =>
    assert.equal(
      view.container.querySelector(".buzz-terminal-welcome") !== null,
      present,
    ),
  );
}

async function reveal(view) {
  const substrate = view.container.querySelector(".buzz-terminal-substrate");
  toggleChord();
  await waitFor(() =>
    assert.equal(substrate.dataset.terminalOwner, "terminal"),
  );
}

test("spawn-time output before the first reveal keeps the welcome overlay", async () => {
  const subject = fixture({ frame: EMPTY_FRAME });
  await ready(subject.view);
  await expectWelcome(subject.view, true);

  subject.rerender({ frame: VISIBLE_FRAME });
  await expectWelcome(subject.view, true);

  await reveal(subject.view);
  await expectWelcome(subject.view, true);
});

test("the first keystroke dismisses the welcome overlay", async () => {
  const subject = fixture({ frame: VISIBLE_FRAME });
  await ready(subject.view);
  await reveal(subject.view);
  await expectWelcome(subject.view, true);

  fireEvent.input(subject.view.getByLabelText("Terminal input"), {
    target: { value: "l" },
  });
  await expectWelcome(subject.view, false);
});

test("non-empty output after the reveal dismisses the welcome overlay", async () => {
  const subject = fixture({ frame: EMPTY_FRAME });
  await ready(subject.view);
  await reveal(subject.view);
  await expectWelcome(subject.view, true);

  subject.rerender({ frame: VISIBLE_FRAME });
  await expectWelcome(subject.view, false);
});

test("empty active output keeps the welcome overlay", async () => {
  const subject = fixture({ frame: EMPTY_FRAME });
  await ready(subject.view);
  await reveal(subject.view);
  await expectWelcome(subject.view, true);

  subject.rerender({
    frame: {
      ...EMPTY_FRAME,
      viewport: { ...EMPTY_FRAME.viewport, generation: 2 },
    },
  });
  await expectWelcome(subject.view, true);
});

test("non-empty output from an inactive PTY keeps the welcome overlay", async () => {
  const subject = fixture({
    sessionFrames: [{ frame: EMPTY_FRAME, sessionId: "one" }],
    sessions: [
      { active: true, closing: false, id: "one", title: "SHELL" },
      { active: false, closing: false, id: "two", title: "LOG" },
    ],
  });
  await ready(subject.view);
  await reveal(subject.view);
  await expectWelcome(subject.view, true);

  subject.rerender({
    sessionFrames: [
      { frame: EMPTY_FRAME, sessionId: "one" },
      { frame: VISIBLE_FRAME, sessionId: "two" },
    ],
  });
  await expectWelcome(subject.view, true);
});

test("mounted wheel path accumulates fractional lines per active session", async () => {
  const { calls, view } = fixture();
  await ready(view);
  const substrate = view.container.querySelector(".buzz-terminal-substrate");
  fireEvent.wheel(substrate, { deltaMode: 0, deltaY: 8 });
  assert.deepEqual(calls.scroll, []);
  fireEvent.wheel(substrate, { deltaMode: 0, deltaY: 10 });
  assert.deepEqual(calls.scroll, [1]);
});

test("canvas failure atomically restores Buzz ownership", async () => {
  const { props, view } = fixture();
  await ready(view);
  const substrate = view.container.querySelector(".buzz-terminal-substrate");
  toggleChord();
  await waitFor(() =>
    assert.equal(substrate.dataset.terminalOwner, "terminal"),
  );
  dom.window.HTMLCanvasElement.prototype.getContext = () => null;
  view.rerender(
    createElement(
      ThemeProvider,
      null,
      createElement("div", {
        className: "buzz-huddle-app-surface",
        tabIndex: -1,
      }),
      createElement(TerminalSubstrate, {
        ...props,
        frame: {
          cursor: { column: 0, line: 0, visible: false },
          full: false,
          rows: [],
          viewport: { columns: 80, generation: 1, screenLines: 24 },
        },
      }),
    ),
  );
  await waitFor(() => assert.equal(substrate.dataset.terminalOwner, "buzz"));
  toggleChord();
  await waitFor(() =>
    assert.equal(substrate.dataset.terminalOwner, "terminal"),
  );
});

test("cursor blink runs only while the terminal owns input and resets on input", async () => {
  const originalSetInterval = window.setInterval;
  const originalClearInterval = window.clearInterval;
  const callbacks = new Map();
  let nextTimer = 1;
  window.setInterval = (callback) => {
    const timer = nextTimer++;
    callbacks.set(timer, callback);
    return timer;
  };
  window.clearInterval = (timer) => callbacks.delete(timer);
  try {
    const subject = fixture({ frame: VISIBLE_FRAME });
    await ready(subject.view);
    assert.equal(callbacks.size, 0, "Buzz ownership must pause blinking");

    toggleChord();
    await waitFor(() => assert.equal(callbacks.size, 1));
    const firstTimer = [...callbacks.keys()][0];
    act(() => callbacks.get(firstTimer)());

    fireEvent.input(subject.view.getByLabelText("Terminal input"), {
      target: { value: "a" },
    });
    await waitFor(() => {
      assert.deepEqual(subject.calls.input, ["a"]);
      assert.equal(callbacks.size, 1);
      assert.equal(callbacks.has(firstTimer), false);
    });

    toggleChord();
    await waitFor(() => assert.equal(callbacks.size, 0));
  } finally {
    window.setInterval = originalSetInterval;
    window.clearInterval = originalClearInterval;
  }
});

test("reduced motion keeps the terminal cursor solid", async () => {
  reducedMotion = true;
  const originalSetInterval = window.setInterval;
  let intervals = 0;
  window.setInterval = () => {
    intervals += 1;
    return 1;
  };
  try {
    const subject = fixture({ frame: VISIBLE_FRAME });
    await ready(subject.view);
    toggleChord();
    await waitFor(() =>
      assert.equal(
        subject.view.container.querySelector(".buzz-terminal-substrate").dataset
          .terminalOwner,
        "terminal",
      ),
    );
    assert.equal(intervals, 0);
  } finally {
    window.setInterval = originalSetInterval;
  }
});

const TWO_SESSIONS = [
  { active: true, closing: false, id: "one", title: "SHELL" },
  { active: false, closing: false, id: "two", title: "LOG" },
];

function frameWith(text, generation = 1) {
  return {
    cursor: { column: 0, line: 0, visible: false },
    full: false,
    rows: [
      {
        line: 0,
        spans: [
          {
            style: { fg: 0, bg: 0, flags: 0 },
            clusters: [{ column: 0, text, width: text.length }],
          },
        ],
      },
    ],
    viewport: { columns: 112, generation, screenLines: 46 },
  };
}

const SWAPPED_SESSIONS = [
  { active: false, closing: false, id: "one", title: "SHELL" },
  { active: true, closing: false, id: "two", title: "LOG" },
];

test("switching back to an idle session repaints its retained rows", async () => {
  const subject = fixture({
    sessionFrames: [
      { frame: frameWith("one"), sessionId: "one" },
      { frame: frameWith("two"), sessionId: "two" },
    ],
    sessions: TWO_SESSIONS,
  });
  await ready(subject.view);
  await waitFor(() =>
    assert.ok(gridDraws(subject.view).some((entry) => entry.text === "one")),
  );

  // A round trip is required, not a single switch: `paint()` is what drains the
  // dirty set, so a grid that has never been painted still holds every line and
  // repaints on first switch by luck. Only a session that was painted and then
  // deactivated has an empty dirty set while holding rows.
  subject.rerender({ sessions: SWAPPED_SESSIONS });
  await waitFor(() =>
    assert.ok(gridDraws(subject.view).some((entry) => entry.text === "two")),
  );

  paintLog.length = 0;
  subject.rerender({ sessions: TWO_SESSIONS });
  await waitFor(() =>
    assert.ok(
      gridDraws(subject.view).some((entry) => entry.text === "one"),
      "returning to an idle session must repaint its retained rows",
    ),
  );
  assert.equal(
    gridDraws(subject.view).some((entry) => entry.text === "two"),
    false,
    "the outgoing session's rows must not be repainted",
  );
});

test("switching to a session with no frame yet clears the outgoing pixels", async () => {
  const subject = fixture({
    sessionFrames: [{ frame: frameWith("one"), sessionId: "one" }],
    sessions: TWO_SESSIONS,
  });
  await ready(subject.view);
  await waitFor(() =>
    assert.ok(gridDraws(subject.view).some((entry) => entry.text === "one")),
  );

  // Session "two" has delivered no frame, so it has no grid: `markAllDirty()`
  // and `paint()` are both no-ops and only a background fill can erase "one".
  paintLog.length = 0;
  subject.rerender({ sessions: SWAPPED_SESSIONS });
  await waitFor(() =>
    assert.ok(
      gridDraws(subject.view).some(
        (entry) =>
          entry.kind === "fill" &&
          entry.x === 0 &&
          entry.y === 0 &&
          entry.width === 940.8 &&
          entry.height === 782,
      ),
      "a full-viewport background fill must erase the outgoing session",
    ),
  );
  assert.equal(
    gridDraws(subject.view).some((entry) => entry.text === "one"),
    false,
  );
});

test("a frame on an unchanged session does not force a full repaint", async () => {
  const subject = fixture({
    sessionFrames: [{ frame: frameWith("one"), sessionId: "one" }],
    sessions: TWO_SESSIONS,
  });
  await ready(subject.view);
  await waitFor(() =>
    assert.ok(gridDraws(subject.view).some((entry) => entry.text === "one")),
  );

  // Guards the damage model, not the bug: dropping the `paintedSessionRef`
  // write leaves `sessionChanged` permanently true, which repaints the whole
  // viewport on every frame. That looks correct on screen and passes every
  // assertion above, so only a negative test can see it.
  paintLog.length = 0;
  subject.rerender({
    sessionFrames: [{ frame: frameWith("more", 1), sessionId: "one" }],
  });
  await waitFor(() =>
    assert.ok(gridDraws(subject.view).some((entry) => entry.text === "more")),
  );
  assert.equal(
    gridDraws(subject.view).some(
      (entry) =>
        entry.kind === "fill" && entry.width === 940.8 && entry.height === 782,
    ),
    false,
    "an incremental frame must not fill the whole viewport",
  );
});

test("a switch during a bailed paint pass is honoured once painting resumes", async () => {
  const subject = fixture({
    sessionFrames: [{ frame: frameWith("one"), sessionId: "one" }],
    sessions: TWO_SESSIONS,
  });
  await ready(subject.view);
  await waitFor(() =>
    assert.ok(gridDraws(subject.view).some((entry) => entry.text === "one")),
  );

  // The paint effect returns early when there is no 2d context. Writing
  // `paintedSessionRef` before those returns would mark this switch as already
  // painted, and the fill that erases "one" would never run.
  dom.window.HTMLCanvasElement.prototype.getContext = () => null;
  subject.rerender({ sessions: SWAPPED_SESSIONS });
  paintLog.length = 0;

  // Restoring the context is not enough to re-run the effect: its deps are the
  // session, cursor, frames and palette. A frame for the now-inactive session
  // re-triggers it without giving "two" a grid and without touching the palette,
  // either of which would force the fill for an unrelated reason.
  dom.window.HTMLCanvasElement.prototype.getContext = function () {
    return paintRecorder(this);
  };
  subject.rerender({
    sessionFrames: [{ frame: frameWith("one", 1), sessionId: "one" }],
    sessions: SWAPPED_SESSIONS,
  });
  await waitFor(() =>
    assert.ok(
      gridDraws(subject.view).some(
        (entry) =>
          entry.kind === "fill" &&
          entry.width === 940.8 &&
          entry.height === 782,
      ),
      "the pending switch must still repaint after the bail",
    ),
  );
});

function tabFixture(overrides = {}) {
  const calls = { close: [], input: [], select: [], spawn: 0 };
  const subject = fixture({
    onCloseSession(id) {
      calls.close.push(id);
    },
    onInput(value) {
      calls.input.push(value);
    },
    onNewSession() {
      calls.spawn += 1;
    },
    onSelectSession(id) {
      calls.select.push(id);
    },
    sessions: [
      { active: true, closing: false, id: "one", title: "SHELL" },
      { active: false, closing: false, id: "two", title: "LOG" },
    ],
    ...overrides,
  });
  // Spread order matters: `fixture` returns its own `calls`, so ours goes last.
  return { ...subject, calls };
}

function press(init) {
  // Dispatched at the window, which is where the substrate's capture-phase
  // listener lives -- firing at the textarea would test React's synthetic
  // tree instead of the layer actually under test.
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  act(() => {
    window.dispatchEvent(event);
  });
  return event;
}

async function revealed(overrides) {
  const subject = tabFixture(overrides);
  await ready(subject.view);
  await reveal(subject.view);
  return subject;
}

test("tab chords drive new, close, and select while the terminal owns input", async () => {
  const subject = await revealed({
    sessions: [
      { active: false, closing: false, id: "one", title: "SHELL" },
      { active: true, closing: false, id: "two", title: "LOG" },
      { active: false, closing: false, id: "three", title: "TAIL" },
    ],
  });

  const spawn = press({ code: "KeyT", metaKey: true });
  assert.equal(subject.calls.spawn, 1);
  assert.equal(spawn.defaultPrevented, true, "⌘T must not reach the page");

  const close = press({ code: "KeyW", metaKey: true });
  assert.deepEqual(subject.calls.close, ["two"]);
  assert.equal(close.defaultPrevented, true, "⌘W must not reach the page");

  press({ code: "ArrowRight", metaKey: true, shiftKey: true });
  press({ code: "ArrowLeft", metaKey: true, shiftKey: true });
  // Three sessions, active in the middle: with only two tabs, prev and next
  // resolve to the same id and a direction swap would pass unnoticed.
  assert.deepEqual(subject.calls.select, ["three", "one"]);

  // Nothing was mistaken for terminal input along the way.
  assert.deepEqual(subject.calls.input, []);
});

test("tab chords stay inert while Buzz owns input", async () => {
  const subject = tabFixture();
  await ready(subject.view);
  // Deliberately not revealed: owner is "buzz".
  const spawn = press({ code: "KeyT", metaKey: true });
  press({ code: "KeyW", metaKey: true });
  press({ code: "ArrowRight", metaKey: true, shiftKey: true });

  assert.equal(subject.calls.spawn, 0);
  assert.deepEqual(subject.calls.close, []);
  assert.deepEqual(subject.calls.select, []);
  assert.equal(
    spawn.defaultPrevented,
    false,
    "Buzz-mode ⌘T belongs to the rest of the app",
  );
});

test("control chords keep reaching the PTY instead of the tab layer", async () => {
  const subject = await revealed();
  const werase = press({ code: "KeyW", ctrlKey: true });

  assert.deepEqual(subject.calls.close, [], "^W is werase, not close-tab");
  assert.equal(subject.calls.spawn, 0);
  assert.equal(
    werase.defaultPrevented,
    false,
    "the chord layer must not consume ^W before the textarea encodes it",
  );
});

test("⌘W on an already-closing tab does not re-fire close", async () => {
  const subject = await revealed({
    sessions: [
      { active: true, closing: true, id: "one", title: "SHELL" },
      { active: false, closing: false, id: "two", title: "LOG" },
    ],
  });
  const close = press({ code: "KeyW", metaKey: true });

  assert.deepEqual(subject.calls.close, []);
  assert.equal(
    close.defaultPrevented,
    true,
    "the chord is still ours -- swallowed, not forwarded",
  );
});

test("the handoff chord still toggles with the tab layer installed", async () => {
  const subject = tabFixture();
  await ready(subject.view);
  const substrate = subject.view.container.querySelector(
    ".buzz-terminal-substrate",
  );
  toggleChord();
  await waitFor(() =>
    assert.equal(substrate.dataset.terminalOwner, "terminal"),
  );
  toggleChord();
  await waitFor(() => assert.equal(substrate.dataset.terminalOwner, "buzz"));
});

// ---------------------------------------------------------------------------
// Splash animation lifecycle.
//
// This substrate is mounted unconditionally on every route and merely
// CSS-concealed in Buzz mode, so "is the splash animating?" is a question about
// `owner`, not about mounting. A loop gated only on `welcomeVisible` ran at
// 120 rAF/s behind the whole app forever for anyone who never opened the
// terminal; that is the defect these arms exist to keep dead.
//
// The rAF clock is driven by hand rather than by jsdom's visual loop: a real
// clock can only show "frames happened", while a manual one can advance AFTER a
// transition and prove no successor callback was scheduled. Distinguishing a
// cancelled frame from a frame that was never scheduled needs that.
function splashClock() {
  const real = {
    request: dom.window.requestAnimationFrame,
    cancel: dom.window.cancelAnimationFrame,
  };
  const pending = new Map();
  let nextHandle = 1;
  let scheduled = 0;
  let cancelled = 0;
  dom.window.requestAnimationFrame = (callback) => {
    const handle = nextHandle++;
    pending.set(handle, callback);
    scheduled += 1;
    return handle;
  };
  dom.window.cancelAnimationFrame = (handle) => {
    if (pending.delete(handle)) cancelled += 1;
  };
  return {
    get scheduled() {
      return scheduled;
    },
    get cancelled() {
      return cancelled;
    },
    get outstanding() {
      return pending.size;
    },
    /** Run every queued callback once, as one frame would. */
    advance(now = 16) {
      const due = [...pending.entries()];
      pending.clear();
      act(() => {
        for (const [, callback] of due) callback(now);
      });
      return due.length;
    },
    restore() {
      dom.window.requestAnimationFrame = real.request;
      dom.window.cancelAnimationFrame = real.cancel;
    },
  };
}

/** Draws issued to the banner/splash canvas only. */
function bannerDraws(view) {
  const banner = view.container.querySelector(".buzz-terminal-welcome");
  if (!banner) return [];
  return paintLog.filter((entry) => entry.canvas === banner);
}

test("the splash animation runs only while the terminal is revealed", async () => {
  const clock = splashClock();
  try {
    // ARM 1 — concealed entry. `enabled` defaults true, so this is the
    // production-shaped state that used to animate behind the channel view.
    const subject = fixture();
    await ready(subject.view);
    const substrate = subject.view.container.querySelector(
      ".buzz-terminal-substrate",
    );
    assert.equal(substrate.dataset.terminalOwner, "buzz");
    await expectWelcome(subject.view, true);
    assert.equal(
      clock.scheduled,
      0,
      "ARM 1: no splash frame is scheduled while Buzz owns the screen",
    );
    clock.advance();
    assert.equal(
      bannerDraws(subject.view).length,
      0,
      "ARM 1: and no hidden banner paint happens either",
    );

    // ARM 2 — revealed. The positive control: the loop must actually run, or
    // arms 1/3 are satisfied by a loop that is simply broken everywhere.
    await reveal(subject.view);
    assert.ok(
      clock.scheduled > 0,
      "ARM 2: revealing the terminal starts the splash loop",
    );
    const afterReveal = clock.scheduled;
    assert.equal(clock.advance(), 1, "ARM 2: exactly one frame was pending");
    assert.ok(
      bannerDraws(subject.view).length > 0,
      "ARM 2: the revealed splash actually paints",
    );
    assert.ok(
      clock.scheduled > afterReveal,
      "ARM 2: the loop reschedules itself while revealed",
    );

    // ARM 3 — terminal -> buzz. The regression users hit: leaving the terminal
    // must CANCEL the outstanding frame, not merely stop new ones. Advancing
    // the clock afterwards is what separates those two.
    const beforeConceal = clock.scheduled;
    const cancelledBefore = clock.cancelled;
    toggleChord();
    await waitFor(() => assert.equal(substrate.dataset.terminalOwner, "buzz"));
    assert.ok(
      clock.cancelled > cancelledBefore,
      "ARM 3: concealing cancels the frame that was already scheduled",
    );
    assert.equal(
      clock.outstanding,
      0,
      "ARM 3: nothing is left queued after cleanup",
    );
    const drawsBefore = bannerDraws(subject.view).length;
    clock.advance();
    clock.advance();
    assert.equal(
      clock.scheduled,
      beforeConceal,
      "ARM 3: no successor callback is scheduled after concealing",
    );
    assert.equal(
      bannerDraws(subject.view).length,
      drawsBefore,
      "ARM 3: and no further hidden banner paint occurs",
    );

    // ARM 4 — buzz -> terminal again. Proves cleanup did not poison the
    // positive path: a fix that permanently kills the loop passes 1 and 3.
    await reveal(subject.view);
    assert.ok(
      clock.scheduled > beforeConceal,
      "ARM 4: re-revealing restarts the splash loop",
    );
    clock.advance();
    assert.ok(
      bannerDraws(subject.view).length > drawsBefore,
      "ARM 4: and it paints again",
    );
  } finally {
    clock.restore();
  }
});

// The gate above reads `owner` alone, which is only sound because
// `owner === "terminal"` implies `enabled`: the sole `commitOwner("terminal")`
// call site sits behind an `!enabled` early return, and dropping `enabled`
// forces ownership back to Buzz. That implication is true by construction
// today and nothing else in this file pins it, so a refactor adding a second
// reveal path outside the `enabled` guard would silently widen the gate. This
// arm is that implication, held as a regression test in both directions.
test("a disabled terminal cannot reveal, so owner-gating cannot widen", async () => {
  const clock = splashClock();
  try {
    const subject = fixture({ enabled: false });
    await ready(subject.view);
    const substrate = subject.view.container.querySelector(
      ".buzz-terminal-substrate",
    );

    toggleChord();
    await waitFor(() => assert.ok(substrate.dataset.terminalOwner));
    assert.equal(
      substrate.dataset.terminalOwner,
      "buzz",
      "the toggle chord must not reveal a terminal that has no session",
    );
    assert.equal(
      clock.scheduled,
      0,
      "and no splash frame is scheduled while disabled",
    );
    clock.advance();
    assert.equal(
      bannerDraws(subject.view).length,
      0,
      "nor any hidden banner paint",
    );

    // The other direction: losing `enabled` while revealed must concede
    // ownership and cancel the loop, which is what makes the `enabled` term
    // redundant in the animation gate rather than merely absent from it.
    subject.rerender({ enabled: true });
    await reveal(subject.view);
    assert.ok(clock.scheduled > 0, "the enabled terminal does reveal and run");
    const afterReveal = clock.scheduled;

    subject.rerender({ enabled: false });
    await waitFor(() => assert.equal(substrate.dataset.terminalOwner, "buzz"));
    assert.equal(
      clock.outstanding,
      0,
      "losing the session cancels the outstanding splash frame",
    );
    clock.advance();
    clock.advance();
    assert.equal(
      clock.scheduled,
      afterReveal,
      "and schedules no successor once disabled",
    );
  } finally {
    clock.restore();
  }
});
