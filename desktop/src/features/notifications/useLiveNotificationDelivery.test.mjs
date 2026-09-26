import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    isTauri: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { platform: "Win32", userAgent: "buzz-test" },
  });
  window.Notification = { permission: "granted" };
});
after(() => dom.window.close());

const { useLiveNotificationDelivery } = await import(
  "./useLiveNotificationDelivery.ts"
);

async function mount(t, initial = {}) {
  const { act, renderHook } = await import("@testing-library/react");
  let permission = Promise.resolve("granted");
  let fails = false;
  const posts = [];
  const sounds = [];
  window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "windows_notification_permission_state")
        return permission;
      assert.equal(command, "show_native_notification");
      posts.push(args.title);
      return fails
        ? Promise.reject(new Error("WinRT unavailable"))
        : Promise.resolve();
    },
  };
  const props = {
    scope: t.name,
    enabled: true,
    dmEnabled: true,
    threadReplyEnabled: true,
    onDelivered: (item) => sounds.push(item.id),
    ...initial,
  };
  const hook = renderHook((options) => useLiveNotificationDelivery(options), {
    initialProps: props,
  });
  t.after(() => hook.unmount());
  return {
    hook,
    props,
    act,
    posts,
    sounds,
    setPermission(value) {
      permission = value;
    },
    setFails(value) {
      fails = value;
    },
    enqueue(slot = "dm") {
      return act(async () => {
        hook.result.current({
          id: "event",
          slot,
          payload: { title: "hello" },
        });
      });
    },
    journal() {
      return JSON.parse(
        window.localStorage.getItem(
          `buzz-live-notification-retry.v1:${props.scope}`,
        ),
      );
    },
  };
}

for (const slot of ["dm", "thread_reply"]) {
  for (const change of ["toggle", "slot", "scope", "unmount"]) {
    test(`${slot} permission continuation is fenced by ${change}`, async (t) => {
      const harness = await mount(t);
      let release;
      harness.setPermission(
        new Promise((resolve) => {
          release = resolve;
        }),
      );
      await harness.enqueue(slot);
      assert.equal(harness.journal().length, 1);
      if (change === "unmount") harness.hook.unmount();
      else
        harness.hook.rerender({
          ...harness.props,
          ...(change === "scope"
            ? { scope: "other-community" }
            : change === "slot"
              ? { [slot === "dm" ? "dmEnabled" : "threadReplyEnabled"]: false }
              : { enabled: false }),
        });
      await harness.act(async () => {
        release("granted");
      });
      assert.deepEqual(harness.posts, []);
      assert.deepEqual(harness.sounds, []);
      assert.equal(harness.journal().length, 1);
    });
  }
}

test("a failed native live alert is durable and resumes after remount", async (t) => {
  const harness = await mount(t);
  harness.setFails(true);
  await harness.enqueue();
  assert.equal(harness.posts.length, 1);
  assert.equal(harness.sounds.length, 0);
  assert.equal(harness.journal().length, 1);
  harness.hook.unmount();
  const restored = await mount(t);
  await restored.act(async () => {});
  assert.equal(restored.posts.length, 1);
  assert.deepEqual(restored.sounds, ["event"]);
  assert.deepEqual(restored.journal(), []);
});

test("retries use backoff and stop after five attempts while retaining the journal", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
  const harness = await mount(t);
  harness.setFails(true);
  await harness.enqueue();
  for (const delay of [1_000, 2_000, 4_000, 8_000, 16_000, 30_000]) {
    await harness.act(async () => {
      t.mock.timers.tick(delay);
    });
  }
  assert.equal(harness.posts.length, 5);
  assert.equal(harness.journal().length, 1);
});

test("a disable during the final send permission query prevents native posting", async (t) => {
  const harness = await mount(t);
  let checks = 0;
  let release;
  const pending = new Promise((resolve) => {
    release = resolve;
  });
  window.__TAURI_INTERNALS__.invoke = (command) => {
    assert.equal(
      command,
      "windows_notification_permission_state",
      "must not reach native posting",
    );
    checks += 1;
    return checks === 1 ? Promise.resolve("granted") : pending;
  };
  await harness.enqueue();
  assert.equal(checks, 2);
  harness.hook.rerender({ ...harness.props, enabled: false });
  await harness.act(async () => {
    release("granted");
  });
  assert.equal(harness.journal().length, 1);
  assert.deepEqual(harness.sounds, []);
});

test("pending live alerts are deduplicated and bounded before persistence", async (t) => {
  const harness = await mount(t);
  harness.setPermission(new Promise(() => {}));
  await harness.act(async () => {
    for (let index = 0; index < 505; index++) {
      const notification = {
        id: `event-${index}`,
        slot: "dm",
        payload: { title: `alert-${index}` },
      };
      harness.hook.result.current(notification);
      harness.hook.result.current(notification);
    }
  });
  const journal = harness.journal();
  assert.equal(journal.length, 500);
  assert.equal(new Set(journal.map((item) => item.id)).size, 500);
  assert.equal(journal[0].id, "event-5");
  assert.equal(journal.at(-1).id, "event-504");
});
