import assert from "node:assert/strict";
import test from "node:test";

const target = {
  kind: "channel-message",
  channelId: "general",
  messageId: "message-a",
  threadRootId: "thread-a",
};

const { allowNavigation, registerNavigationGuard, traverseHistory } =
  await import("./navigationGuard.ts");

test("all navigation consults the registered boundary guard", () => {
  let received;
  const unregister = registerNavigationGuard((nextTarget) => {
    received = nextTarget;
    return false;
  });

  assert.equal(allowNavigation(target), false);
  assert.deepEqual(received, target);
  unregister();
  assert.equal(allowNavigation(target), true);
});

test("guarded history traversal blocks before mutating history", () => {
  let received;
  let backCalls = 0;
  const unregister = registerNavigationGuard((nextTarget) => {
    received = nextTarget;
    return false;
  });

  assert.equal(
    traverseHistory(
      {
        back: () => {
          backCalls += 1;
        },
        forward: () => {},
      },
      "back",
    ),
    false,
  );
  assert.deepEqual(received, { kind: "history", direction: "back" });
  assert.equal(backCalls, 0);
  unregister();
});

test("guarded history traversal invokes the selected direction when allowed", () => {
  let forwardCalls = 0;

  assert.equal(
    traverseHistory(
      {
        back: () => {},
        forward: () => {
          forwardCalls += 1;
        },
      },
      "forward",
    ),
    true,
  );
  assert.equal(forwardCalls, 1);
});

test("unregistering the newer guard restores the prior live guard", () => {
  const unregisterFirst = registerNavigationGuard(() => false);
  const unregisterSecond = registerNavigationGuard(() => true);

  assert.equal(allowNavigation(target), true);
  unregisterSecond();
  assert.equal(allowNavigation(target), false);
  unregisterFirst();
  assert.equal(allowNavigation(target), true);
});

test("stale cleanup cannot unregister a newer guard", () => {
  const unregisterFirst = registerNavigationGuard(() => false);
  const unregisterSecond = registerNavigationGuard(() => true);

  unregisterFirst();
  assert.equal(allowNavigation(target), true);
  unregisterSecond();
  assert.equal(allowNavigation(target), true);
});

test("duplicate callback registrations clean up by registration identity", () => {
  const sharedGuard = () => false;
  const unregisterFirst = registerNavigationGuard(sharedGuard);
  const unregisterSecond = registerNavigationGuard(sharedGuard);

  unregisterFirst();
  assert.equal(allowNavigation(target), false);
  unregisterSecond();
  assert.equal(allowNavigation(target), true);
});
