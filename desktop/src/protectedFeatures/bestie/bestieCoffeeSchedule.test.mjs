import assert from "node:assert/strict";
import test from "node:test";

import {
  formatBestieCoffeeRunPrompt,
  coffeeScheduleGate,
  deriveBestieCoffeeBrief,
  isAtOrPastCoffeeTime,
  isWithinCoffeeScheduleWindow,
  isBestieAgentOnlineForCoffee,
  localDayKey,
  nextCoffeeDueAt,
} from "./bestieCoffeeSchedule.ts";
import { emptyBestieCoffeeState } from "./bestieCoffeeStorage.ts";

test("online check is presence === online only", () => {
  assert.equal(isBestieAgentOnlineForCoffee("online"), true);
  assert.equal(isBestieAgentOnlineForCoffee("away"), false);
  assert.equal(isBestieAgentOnlineForCoffee("offline"), false);
  assert.equal(isBestieAgentOnlineForCoffee(null), false);
});

test("schedule gate skips offline, before time, duplicate day, brewing", () => {
  // Local 10:00 on a fixed day
  const now = Math.floor(new Date(2026, 8, 27, 10, 0, 0).getTime() / 1000);
  let state = emptyBestieCoffeeState();
  state = {
    ...state,
    prefs: { hour: 8, minute: 0 },
  };

  assert.equal(
    coffeeScheduleGate({ isAgentOnline: false, nowSeconds: now, state }).reason,
    "offline",
  );

  const morning = Math.floor(new Date(2026, 8, 27, 7, 30, 0).getTime() / 1000);
  assert.equal(
    coffeeScheduleGate({
      isAgentOnline: true,
      nowSeconds: morning,
      state,
    }).reason,
    "before-schedule",
  );

  assert.equal(
    coffeeScheduleGate({ isAgentOnline: true, nowSeconds: now, state }).reason,
    "missed-window",
  );

  const atEight = Math.floor(new Date(2026, 8, 27, 8, 0, 0).getTime() / 1000);
  assert.equal(
    coffeeScheduleGate({ isAgentOnline: true, nowSeconds: atEight, state }).ok,
    true,
  );
  const atEight04 = Math.floor(new Date(2026, 8, 27, 8, 4, 0).getTime() / 1000);
  assert.equal(
    coffeeScheduleGate({ isAgentOnline: true, nowSeconds: atEight04, state }).ok,
    true,
  );
  const atEight06 = Math.floor(new Date(2026, 8, 27, 8, 6, 0).getTime() / 1000);
  assert.equal(
    coffeeScheduleGate({ isAgentOnline: true, nowSeconds: atEight06, state }).reason,
    "missed-window",
  );

  state = {
    ...state,
    lastScheduledDayKey: localDayKey(now),
  };
  assert.equal(
    coffeeScheduleGate({ isAgentOnline: true, nowSeconds: now, state }).reason,
    "already-ran-today",
  );

  state = {
    ...emptyBestieCoffeeState(),
    pendingRun: {
      source: "brew",
      startedAt: now,
      triggerMessageId: null,
    },
    prefs: { hour: 8, minute: 0 },
  };
  assert.equal(
    coffeeScheduleGate({ isAgentOnline: true, nowSeconds: now, state }).reason,
    "brewing",
  );
});

test("isAtOrPastCoffeeTime and nextCoffeeDueAt use local clock", () => {
  const prefs = { hour: 8, minute: 0 };
  const before = Math.floor(new Date(2026, 8, 27, 7, 59, 0).getTime() / 1000);
  const after = Math.floor(new Date(2026, 8, 27, 8, 0, 0).getTime() / 1000);
  assert.equal(isAtOrPastCoffeeTime(prefs, before), false);
  assert.equal(isAtOrPastCoffeeTime(prefs, after), true);
  const next = nextCoffeeDueAt(prefs, after, { afterRun: true });
  assert.ok(next > after);
});

test("deriveBestieCoffeeBrief takes first sentence", () => {
  assert.equal(
    deriveBestieCoffeeBrief("Hello world. More detail here."),
    "Hello world.",
  );
  assert.ok(deriveBestieCoffeeBrief("x".repeat(200)).endsWith("…"));
});

test("formatBestieCoffeeRunPrompt invokes /hula-coffee skill with source tag", () => {
  const brew = formatBestieCoffeeRunPrompt("brew");
  assert.match(brew, /\[Bestie coffee\] · brew/);
  assert.match(brew, /\/hula-coffee/);
  const scheduled = formatBestieCoffeeRunPrompt("scheduled");
  assert.match(scheduled, /\[Bestie coffee\] · scheduled/);
});
