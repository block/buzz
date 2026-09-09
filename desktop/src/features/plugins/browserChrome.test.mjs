import assert from "node:assert/strict";
import test from "node:test";

const {
  applyBrowserEvent,
  applyCommandFailure,
  blurAddress,
  BROWSER_ERROR_MESSAGES,
  createCommandOutcomeTracker,
  createInitialChromeState,
  editAddress,
  eventExplainsCommand,
  eventExplainsCurrentCommand,
  focusAddress,
  normalizeSubmittedAddress,
  PLUGIN_SMOKE_PASSTHROUGH_COMMANDS,
  resyncAfterCommand,
  submitAddress,
} = await import("./browserChrome.ts");

const SESSION = { sessionId: "session-1", generation: 3 };

function loading(overrides = {}) {
  return {
    type: "loading",
    sessionId: SESSION.sessionId,
    generation: SESSION.generation,
    url: "https://example.com/",
    ...overrides,
  };
}
function location(overrides = {}) {
  return {
    type: "location",
    sessionId: SESSION.sessionId,
    generation: SESSION.generation,
    url: "https://example.com/docs",
    canGoBack: true,
    canGoForward: false,
    ...overrides,
  };
}
function loaded(overrides = {}) {
  return {
    type: "loaded",
    sessionId: SESSION.sessionId,
    generation: SESSION.generation,
    url: "https://example.com/docs",
    ...overrides,
  };
}
function errorEvent(code, overrides = {}) {
  return {
    type: "error",
    sessionId: SESSION.sessionId,
    generation: SESSION.generation,
    code,
    ...overrides,
  };
}

test("empty or whitespace address is invalid and never navigates", () => {
  assert.equal(normalizeSubmittedAddress(""), null);
  assert.equal(normalizeSubmittedAddress("   "), null);
  const state = createInitialChromeState("https://example.com/");
  const { state: next, address } = submitAddress(state, "   ");
  assert.equal(address, null);
  assert.equal(next.addressInvalid, true);
});

test("every closed error code renders its fixed text", () => {
  const state = createInitialChromeState("https://example.com/");
  for (const code of Object.keys(BROWSER_ERROR_MESSAGES)) {
    const next = applyBrowserEvent(state, SESSION, errorEvent(code));
    assert.equal(next.errorMessage, BROWSER_ERROR_MESSAGES[code]);
    assert.equal(next.errorCode, code);
    assert.equal(next.loading, false);
  }
});

test("an unrecognized code still renders a fixed fallback, never raw plugin text", () => {
  const state = createInitialChromeState("https://example.com/");
  const next = applyBrowserEvent(state, SESSION, errorEvent("not-a-real-code"));
  assert.equal(next.errorMessage, "Something went wrong.");
});

test("loaded is completion-only: it clears loading and updates the address", () => {
  let state = createInitialChromeState("https://example.com/");
  state = applyBrowserEvent(state, SESSION, loading());
  assert.equal(state.loading, true);
  state = applyBrowserEvent(state, SESSION, loaded());
  assert.equal(state.loading, false);
  assert.equal(state.addressValue, "https://example.com/docs");
  assert.equal(state.errorCode, null);
});

test("location is display-only: it never clears loading state", () => {
  let state = createInitialChromeState("https://example.com/");
  state = applyBrowserEvent(state, SESSION, loading());
  assert.equal(state.loading, true);
  state = applyBrowserEvent(state, SESSION, location());
  assert.equal(
    state.loading,
    true,
    "location must not clear the pending deadline/loading state",
  );
  assert.equal(state.addressValue, "https://example.com/docs");
  assert.equal(state.canGoBack, true);
  assert.equal(state.canGoForward, false);
});

test("a focused, unsubmitted edit is never overwritten by location or loaded", () => {
  let state = createInitialChromeState("https://example.com/");
  state = focusAddress(state);
  state = editAddress(state, "still typing");
  state = applyBrowserEvent(state, SESSION, location());
  assert.equal(state.addressValue, "still typing");
  state = applyBrowserEvent(state, SESSION, loaded());
  assert.equal(state.addressValue, "still typing");
});

test("blur without an edit resyncs the field to the last known URL", () => {
  let state = createInitialChromeState("https://example.com/");
  state = applyBrowserEvent(state, SESSION, loaded());
  state = focusAddress(state);
  // No edit occurred — blur must re-sync, not preserve stale display text.
  state.addressValue = "stale text a user never typed";
  state = blurAddress(state);
  assert.equal(state.addressValue, "https://example.com/docs");
  assert.equal(state.addressFocused, false);
});

test("blur with an edit in progress does not resync", () => {
  let state = createInitialChromeState("https://example.com/");
  state = focusAddress(state);
  state = editAddress(state, "example.org");
  state = blurAddress(state);
  assert.equal(state.addressValue, "example.org");
  assert.equal(state.addressFocused, false);
});

test("submit clears the edit lock so later events can update the field", () => {
  let state = createInitialChromeState("https://example.com/");
  state = focusAddress(state);
  state = editAddress(state, "example.org/docs");
  const { state: submitted, address } = submitAddress(
    state,
    "example.org/docs",
  );
  assert.equal(address, "example.org/docs");
  state = submitted;
  state = applyBrowserEvent(
    state,
    SESSION,
    loaded({ url: "https://example.org/docs" }),
  );
  assert.equal(state.addressValue, "https://example.org/docs");
});

test("back and forward reflect native availability from the location event, not liveness", () => {
  let state = createInitialChromeState("https://example.com/");
  assert.equal(state.canGoBack, false);
  assert.equal(state.canGoForward, false);
  state = applyBrowserEvent(
    state,
    SESSION,
    location({ canGoBack: true, canGoForward: true }),
  );
  assert.equal(state.canGoBack, true);
  assert.equal(state.canGoForward, true);
  state = applyBrowserEvent(
    state,
    SESSION,
    location({ canGoBack: false, canGoForward: true }),
  );
  assert.equal(state.canGoBack, false);
  assert.equal(state.canGoForward, true);
});

test("resyncAfterCommand (back/forward/reload) clears the edit lock but does not force loading", () => {
  let state = createInitialChromeState("https://example.com/");
  state = applyBrowserEvent(state, SESSION, loaded());
  state = focusAddress(state);
  state = editAddress(state, "unsent edit");
  state = resyncAfterCommand(state);
  assert.equal(state.addressValue, "https://example.com/docs");
  assert.equal(state.addressEdited, false);
  assert.equal(
    state.loading,
    false,
    "only the native loading event may arm the spinner",
  );
});

test("a same-document back/forward traversal (location only, no loading/loaded) never leaves a stuck spinner", () => {
  let state = createInitialChromeState("https://example.com/");
  state = applyBrowserEvent(state, SESSION, loaded());
  state = resyncAfterCommand(state); // handleBack's pre-invoke resync
  assert.equal(state.loading, false);
  state = applyBrowserEvent(
    state,
    SESSION,
    location({ url: "https://example.com/docs#section" }),
  );
  assert.equal(
    state.loading,
    false,
    "fragment/history traversal has no loaded event to clear a spinner that was never armed",
  );
  assert.equal(state.addressValue, "https://example.com/docs#section");
});

test("a stale generation is ignored entirely", () => {
  const state = createInitialChromeState("https://example.com/");
  const stale = applyBrowserEvent(
    state,
    SESSION,
    loaded({ generation: SESSION.generation - 1 }),
  );
  assert.deepStrictEqual(stale, state);
  const foreignSession = applyBrowserEvent(
    state,
    SESSION,
    loaded({ sessionId: "some-other-session" }),
  );
  assert.deepStrictEqual(foreignSession, state);
});

test("a refusal leaves the typed address visible — it is never reverted to home", () => {
  // Public contract (docs/plugin-browser-prototype.md): "A typed address
  // stays visible in the address field" after a `navigation-denied` refusal.
  let state = createInitialChromeState("https://example.com/");
  state = focusAddress(state);
  state = editAddress(state, "not a web address");
  const { state: submitted, address } = submitAddress(
    state,
    "not a web address",
  );
  assert.equal(address, "not a web address");
  state = submitted;
  state = applyBrowserEvent(state, SESSION, errorEvent("navigation-denied"));
  assert.equal(state.addressValue, "not a web address");
  assert.equal(state.errorCode, "navigation-denied");
});

test("applyCommandFailure always renders the generic fallback regardless of loading", () => {
  // Precedence (domain event vs. generic fallback, stale vs. current
  // dispatch) is enforced by the caller via `createCommandOutcomeTracker`,
  // not by this function — `loading` is not a reliable "already explained"
  // signal since a location-only traversal never sets it.
  let state = createInitialChromeState("https://example.com/");
  state = applyBrowserEvent(state, SESSION, loaded()); // loading now false
  state = applyCommandFailure(state);
  assert.equal(state.errorCode, "plugin-unavailable");
  assert.equal(state.loading, false);
});

test("a late domain event still overwrites an earlier command-rejection fallback", () => {
  // The opposite race: the invoke rejection settles before the event
  // arrives. The fallback fires first, but the event must still win once it
  // lands, since `applyBrowserEvent`'s error case does not check `loading`.
  let state = createInitialChromeState("https://example.com/");
  state = resyncAfterCommand(state);
  state = applyCommandFailure(state);
  assert.equal(state.errorCode, "plugin-unavailable");
  state = applyBrowserEvent(state, SESSION, errorEvent("navigation-denied"));
  assert.equal(state.errorCode, "navigation-denied");
});

test("createCommandOutcomeTracker: a domain event marks the current dispatch explained, suppressing the fallback", () => {
  const tracker = createCommandOutcomeTracker();
  const token = tracker.dispatch();
  tracker.markExplained();
  assert.equal(tracker.shouldReportFailure(token), false);
});

test("createCommandOutcomeTracker: a rejection with no preceding domain event still reports failure, even with nothing else loading", () => {
  const tracker = createCommandOutcomeTracker();
  const token = tracker.dispatch();
  assert.equal(tracker.shouldReportFailure(token), true);
});

test("createCommandOutcomeTracker: a stale dispatch's rejection is superseded once a newer command is dispatched", () => {
  const tracker = createCommandOutcomeTracker();
  const staleToken = tracker.dispatch();
  const currentToken = tracker.dispatch();
  assert.equal(
    tracker.shouldReportFailure(staleToken),
    false,
    "superseded by the newer dispatch",
  );
  assert.equal(tracker.shouldReportFailure(currentToken), true);
});

test("eventExplainsCommand: only error explains a dispatched command's rejection — loading, location, and loaded do not", () => {
  assert.equal(eventExplainsCommand("error"), true);
  assert.equal(eventExplainsCommand("loading"), false);
  assert.equal(eventExplainsCommand("location"), false);
  assert.equal(eventExplainsCommand("loaded"), false);
});

test("a display-only location poll before an invoke rejection does not suppress the fallback", () => {
  // location fires independent of any dispatched command (a periodic poll),
  // so it must never be treated as "this attempt is explained" — doing so
  // could mask a genuine invoke failure that arrives afterward.
  const tracker = createCommandOutcomeTracker();
  const token = tracker.dispatch();
  if (eventExplainsCommand("location")) tracker.markExplained();
  assert.equal(tracker.shouldReportFailure(token), true);
});

test("a loaded event for a prior document before a newer invoke's rejection does not suppress the fallback", () => {
  // `loaded` can belong to a document that was already in flight when the
  // user submitted a new address — it says nothing about that newer,
  // still-pending invoke, so it must not explain its eventual rejection.
  const tracker = createCommandOutcomeTracker();
  const token = tracker.dispatch();
  if (eventExplainsCommand("loaded")) tracker.markExplained();
  assert.equal(tracker.shouldReportFailure(token), true);
});

test("eventExplainsCurrentCommand: a wrong-session error does not suppress the fallback for the live dispatch", () => {
  // The exact scenario BrowserSurface's handleIncoming must get right: the
  // user dispatches a command on the live session, a *stale* session's error
  // event lands (a leftover from an earlier open/route), and the live
  // command's own invoke then rejects. The stale event must not explain it.
  const tracker = createCommandOutcomeTracker();
  const token = tracker.dispatch();
  const staleEvent = errorEvent("navigation-denied", {
    generation: SESSION.generation - 1,
  });
  if (eventExplainsCurrentCommand(SESSION, staleEvent)) {
    tracker.markExplained();
  }
  assert.equal(
    tracker.shouldReportFailure(token),
    true,
    "a wrong-generation error must not suppress the fallback",
  );

  let state = createInitialChromeState("https://example.com/");
  state = resyncAfterCommand(state);
  if (tracker.shouldReportFailure(token)) {
    state = applyCommandFailure(state);
  }
  assert.equal(
    state.errorCode,
    "plugin-unavailable",
    "the fallback must still be visible — the stale event explained nothing",
  );
});

test("eventExplainsCurrentCommand: a matching-session error does suppress the fallback for the live dispatch", () => {
  const tracker = createCommandOutcomeTracker();
  const token = tracker.dispatch();
  const matchingEvent = errorEvent("navigation-denied");
  if (eventExplainsCurrentCommand(SESSION, matchingEvent)) {
    tracker.markExplained();
  }
  assert.equal(
    tracker.shouldReportFailure(token),
    false,
    "a same-session error must suppress the fallback",
  );

  let state = createInitialChromeState("https://example.com/");
  state = resyncAfterCommand(state);
  state = applyBrowserEvent(state, SESSION, matchingEvent);
  if (tracker.shouldReportFailure(token)) {
    state = applyCommandFailure(state);
  }
  assert.equal(
    state.errorCode,
    "navigation-denied",
    "the domain error must survive — the invoke rejection's fallback never applied",
  );
});

test("the smoke passthrough set carries exactly the fourteen plugin_* commands and the four event commands", () => {
  const pluginCommands = [...PLUGIN_SMOKE_PASSTHROUGH_COMMANDS].filter((c) =>
    c.startsWith("plugin_"),
  );
  const eventCommands = [...PLUGIN_SMOKE_PASSTHROUGH_COMMANDS].filter((c) =>
    c.startsWith("plugin:event|"),
  );
  assert.equal(pluginCommands.length, 14);
  assert.equal(eventCommands.length, 4);
  assert.equal(PLUGIN_SMOKE_PASSTHROUGH_COMMANDS.size, 18);
  assert.ok(!PLUGIN_SMOKE_PASSTHROUGH_COMMANDS.has("get_identity"));
});
