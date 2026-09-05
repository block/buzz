import assert from "node:assert/strict";
import test from "node:test";
import { lifecycleClient, receiveLifecycle } from "./desktopLifecycle.ts";
const scope = { owner: "owner", community: "wss://one.example" };
const tick = () => new Promise((resolve) => setImmediate(resolve));
function fixture() {
  let epoch = 0,
    connection = 1,
    active = true,
    live;
  let placement = ["source", "selection"],
    stopOutcome = "stopped";
  let lifecycleOutcome = "running",
    history = [],
    ackLost = false;
  const prepared = [],
    sent = [],
    calls = [],
    errors = [];
  const ipc = async (command, args) => {
    assert.equal(args.owner, scope.owner);
    assert.equal(args.community, scope.community);
    calls.push([command, args]);
    if (command === "observe_desktop_placement") return;
    if (command === "read_desktop_placement") return placement;
    if (
      command === "prepare_desktop_lifecycle" ||
      command === "prepare_desktop_stop"
    ) {
      const request = {
        id: `request-${prepared.length}`,
        kind: command.endsWith("_stop") ? 50180 : 50182,
        ...args,
      };
      prepared.push(request);
      return request;
    }
    if (command === "read_desktop_lifecycle_results")
      return args.request.action === "status" ? "running" : lifecycleOutcome;
    if (command === "read_desktop_stop_results") return stopOutcome;
    if (command === "receive_desktop_lifecycle")
      return { id: "result", kind: 50183 };
    throw Error(command);
  };
  const relay = {
    getSessionEpoch: () => epoch,
    getConnectionGeneration: () => connection,
    fetchEvents: async (filter) =>
      filter.kinds.includes(50182) ? history : [],
    publishEvent: async (event, _timeout, _failure, check) => {
      check();
      sent.push(event);
      if (ackLost) throw Error("ACK lost");
    },
    subscribeLive: async (filter, callback) => {
      assert.deepEqual(filter, {
        kinds: [50182, 50180],
        authors: [scope.owner],
        limit: 0,
      });
      live = callback;
      return () => {
        live = undefined;
      };
    },
  };
  return {
    ipc,
    relay,
    prepared,
    sent,
    calls,
    errors,
    client: () => lifecycleClient(scope, () => active, ipc, relay),
    changeScope: () => {
      epoch++;
    },
    disconnect: () => {
      connection++;
    },
    unmount: () => {
      active = false;
    },
    stop: (value) => {
      stopOutcome = value;
    },
    outcome: (value) => {
      lifecycleOutcome = value;
    },
    place: (value) => {
      placement = value;
    },
    history: (value) => {
      history = value;
    },
    loseAck: () => {
      ackLost = true;
    },
    deliver: (event) => live?.(event),
  };
}

test("lost ACK/result permits explicit exact-byte retry, not a fresh Start", async () => {
  const f = fixture(),
    client = f.client();
  const request = await client.start("destination", "agent");
  f.loseAck();
  f.outcome("unknown");
  assert.equal(await client.send(request, 0), "unknown");
  f.outcome("running");
  assert.equal(await client.send(request, 1), "running");
  assert.equal(f.prepared.length, 1);
  assert.deepEqual(f.sent, [request, request]);
  assert.equal(f.sent[0], f.sent[1]);
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    0,
  );
});

for (const interruption of ["changeScope", "disconnect", "unmount"]) {
  test(`${interruption} during Stop cancels Move before destination prepare or send`, async () => {
    const f = fixture(),
      client = f.client();
    const publish = f.relay.publishEvent;
    f.relay.publishEvent = async (...args) => {
      await publish(...args);
      if (args[0].kind === 50180) f[interruption]();
    };
    await assert.rejects(
      client.move("agent", "destination", ["source"], () => {}),
      /scope changed/,
    );
    assert.equal(f.prepared.filter((r) => r.action === "start").length, 0);
  });
}

test("failed Stop is final even if a successful outcome appears later", async () => {
  const f = fixture();
  f.stop("failed");
  await assert.rejects(
    f.client().move("agent", "destination", ["source"], () => {}),
    /will not continue later/,
  );
  f.stop("stopped");
  await tick();
  assert.equal(f.prepared.filter((r) => r.action === "start").length, 0);
});

test("unconfirmed Stop exhausts polling without storing any future Start", async () => {
  const f = fixture();
  f.stop("unknown");
  const timer = globalThis.setTimeout;
  globalThis.setTimeout = (fn) => {
    queueMicrotask(fn);
    return 0;
  };
  try {
    await assert.rejects(
      f.client().move("agent", "destination", ["source"], () => {}),
      /Could not confirm source Stop/,
    );
    assert.equal(f.prepared.filter((r) => r.action === "start").length, 0);
  } finally {
    globalThis.setTimeout = timer;
  }
});

test("Move dispatches Start only after Stop success and unchanged placement", async () => {
  const f = fixture();
  assert.equal(
    await f.client().move("agent", "destination", ["source"], () => {}),
    "running",
  );
  assert.deepEqual(
    f.sent.map((r) => r.action ?? "stop"),
    ["status", "stop", "start"],
  );
  assert.equal(f.sent.at(-1).desktop, "destination");
  const stopRead = f.calls.findIndex(
    ([c]) => c === "read_desktop_stop_results",
  );
  const startPrepare = f.calls.findIndex(
    ([c, a]) => c === "prepare_desktop_lifecycle" && a.action === "start",
  );
  assert.ok(stopRead < startPrepare);
});

test("another Desktop's new placement supersedes an in-flight Move", async () => {
  const f = fixture(),
    publish = f.relay.publishEvent;
  f.relay.publishEvent = async (...args) => {
    await publish(...args);
    if (args[0].kind === 50180) f.place(["third", "new-selection"]);
  };
  await assert.rejects(
    f.client().move("agent", "destination", ["source"], () => {}),
    /Placement changed/,
  );
  assert.equal(f.prepared.filter((r) => r.action === "start").length, 0);
});

test("Restart resolves current host and binds its Status observation, not destination", async () => {
  const f = fixture();
  const request = await f.client().restart("agent", ["source", "other"]);
  assert.equal(request.desktop, "source");
  assert.equal(request.observed, f.prepared[0].id);
  assert.equal(request.action, "restart");
});

test("receiver projects history without executing it and invalidates live work on disconnect", async () => {
  const f = fixture();
  f.history([{ id: "historical", kind: 50182 }]);
  const close = await receiveLifecycle(
    scope,
    () => true,
    (e) => f.errors.push(e),
    f.ipc,
    f.relay,
  );
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    0,
  );
  f.deliver({ id: "live", kind: 50182 });
  await tick();
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    1,
  );
  f.disconnect();
  f.deliver({ id: "late", kind: 50182 });
  await tick();
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    1,
  );
  assert.equal(f.errors.length, 1);
  close();
});

for (const stage of [
  "subscription",
  "history",
  "projection",
  "reconciliation",
]) {
  test(`receiver reports safe ${stage} failure and never admits queued work`, async () => {
    const f = fixture();
    const secret = "private key /home/private bearer secret";
    if (stage === "subscription")
      f.relay.subscribeLive = async () => {
        throw Error(secret);
      };
    if (stage === "history")
      f.relay.fetchEvents = async () => {
        throw Error(secret);
      };
    const ipc = async (command, args) => {
      if (
        command === "observe_desktop_placement" &&
        ((stage === "projection" && !args.reconcile) ||
          (stage === "reconciliation" && args.reconcile))
      )
        throw Error(secret);
      return f.ipc(command, args);
    };
    const started = receiveLifecycle(
      scope,
      () => true,
      (e) => f.errors.push(e),
      ipc,
      f.relay,
    );
    f.deliver({ id: "queued", kind: 50182 });
    await assert.rejects(started, (error) => {
      assert.match(error.message, new RegExp(`${stage}: request failed`));
      assert.doesNotMatch(error.message, /private|bearer|secret/);
      return true;
    });
    await tick();
    assert.equal(
      f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
      0,
    );
    assert.deepEqual(
      f.errors,
      [],
      "discarded callbacks cannot replace the startup diagnosis",
    );
  });
}

test("explicit receiver recovery discards old queued work and projects history without replay", async () => {
  const f = fixture();
  let rejectHistory;
  f.relay.fetchEvents = () =>
    new Promise((_, reject) => {
      rejectHistory = reject;
    });
  const failed = receiveLifecycle(
    scope,
    () => true,
    () => {},
    f.ipc,
    f.relay,
  );
  await tick();
  f.deliver({ id: "old-live", kind: 50182 });
  rejectHistory(Error("Timed out while loading channel history."));
  await assert.rejects(failed, /history: history timed out/);
  f.relay.fetchEvents = async () => [{ id: "old-live", kind: 50182 }];
  const close = await receiveLifecycle(
    scope,
    () => true,
    () => {},
    f.ipc,
    f.relay,
  );
  await tick();
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    0,
  );
  f.deliver({ id: "new-live", kind: 50182 });
  await tick();
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    1,
  );
  f.deliver({ id: "closed-queue", kind: 50182 });
  close();
  await tick();
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    1,
  );
});

test("readiness timeout is distinct, late EOSE recovers, CLOSED retires old callbacks", async () => {
  const f = fixture();
  let notify, deliver;
  let closed = 0,
    ready = 0;
  f.relay.subscribeLive = async (_filter, event, onReady, timeout) => {
    assert.equal(timeout, 5000);
    deliver = event;
    notify = onReady;
    notify("timeout");
    return () => {
      closed++;
    };
  };
  const close = await receiveLifecycle(
    scope,
    () => true,
    (e) => f.errors.push(e),
    f.ipc,
    f.relay,
    () => ready++,
  );
  assert.match(f.errors[0], /readiness timed out/);
  assert.equal(ready, 0);
  notify("eose");
  assert.equal(ready, 1);
  notify("closed");
  deliver({ id: "late", kind: 50182 });
  await tick();
  assert.equal(closed, 1);
  assert.equal(
    f.calls.filter(([c]) => c === "receive_desktop_lifecycle").length,
    0,
  );
  assert.match(f.errors.at(-1), /subscription closed/);
  close();
});
