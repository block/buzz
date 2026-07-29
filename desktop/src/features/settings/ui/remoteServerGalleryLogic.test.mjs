import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  PROVIDER_INFO_UNANSWERED,
  remoteServerEntries,
  remoteServerProbes,
} from "./remoteServerGalleryLogic.ts";

function provider(id) {
  return { id, binaryPath: `/home/u/.local/bin/buzz-backend-${id}` };
}

function okProbe(overrides = {}) {
  return {
    status: "ok",
    result: { ok: true, name: "SSH", version: "0.4.26", ...overrides },
  };
}

describe("remoteServerEntries", () => {
  it("returns nothing when no provider is installed", () => {
    assert.deepEqual(remoteServerEntries([], {}), []);
  });

  it("reads name, version and description off a successful probe", () => {
    const entries = remoteServerEntries([provider("ssh")], {
      ssh: okProbe({ description: "Run agents over SSH." }),
    });
    assert.equal(entries.length, 1);
    assert.equal(entries[0].label, "SSH");
    assert.equal(entries[0].version, "0.4.26");
    assert.equal(entries[0].description, "Run agents over SSH.");
    assert.equal(entries[0].status, "ready");
    assert.equal(entries[0].error, null);
    assert.equal(entries[0].binaryPath, "/home/u/.local/bin/buzz-backend-ssh");
  });

  it("falls back to the id while the probe is still in flight", () => {
    const entries = remoteServerEntries([provider("ssh")], {
      ssh: { status: "loading" },
    });
    assert.equal(entries[0].label, "ssh");
    assert.equal(entries[0].status, "probing");
    assert.equal(entries[0].version, null);
  });

  it("treats a missing probe as still in flight", () => {
    const entries = remoteServerEntries([provider("ssh")], {});
    assert.equal(entries[0].status, "probing");
  });

  it("surfaces a spawn failure on the row instead of dropping it", () => {
    const entries = remoteServerEntries([provider("ssh")], {
      ssh: { status: "failed", error: "binary not found" },
    });
    assert.equal(entries[0].status, "unavailable");
    assert.equal(entries[0].error, "binary not found");
    // Still named and listed — a broken provider the user installed is a fact
    // worth showing, not one to hide.
    assert.equal(entries[0].label, "ssh");
  });

  it("treats an ok:false answer as unavailable", () => {
    const entries = remoteServerEntries([provider("ssh")], {
      ssh: { status: "ok", result: { ok: false } },
    });
    assert.equal(entries[0].status, "unavailable");
    assert.equal(entries[0].error, PROVIDER_INFO_UNANSWERED);
  });

  it("ignores a name carried on an ok:false answer", () => {
    const entries = remoteServerEntries([provider("ssh")], {
      ssh: { status: "ok", result: { ok: false, name: "SSH", version: "9" } },
    });
    assert.equal(entries[0].label, "ssh");
    assert.equal(entries[0].version, null);
  });

  it("blank name/version/description read as absent, not as empty strings", () => {
    const entries = remoteServerEntries([provider("ssh")], {
      ssh: {
        status: "ok",
        result: { ok: true, name: "  ", version: "  ", description: "  " },
      },
    });
    assert.equal(entries[0].label, "ssh");
    assert.equal(entries[0].version, null);
    assert.equal(entries[0].description, null);
  });

  it("sorts ready rows first, then alphabetically, so PATH order cannot reshuffle it", () => {
    const providers = [provider("zeta"), provider("blox"), provider("ssh")];
    const probes = {
      zeta: okProbe({ name: "zeta" }),
      blox: { status: "failed", error: "nope" },
      ssh: okProbe({ name: "SSH" }),
    };
    assert.deepEqual(
      remoteServerEntries(providers, probes).map((entry) => entry.id),
      ["ssh", "zeta", "blox"],
    );
    assert.deepEqual(
      remoteServerEntries([...providers].reverse(), probes).map(
        (entry) => entry.id,
      ),
      ["ssh", "zeta", "blox"],
    );
  });

  it("does not mutate the caller's provider list", () => {
    const providers = [provider("zeta"), provider("blox")];
    const snapshot = providers.map((entry) => entry.id);
    remoteServerEntries(providers, {});
    assert.deepEqual(
      providers.map((entry) => entry.id),
      snapshot,
    );
  });
});

describe("remoteServerProbes", () => {
  it("reads a pending query as still in flight", () => {
    assert.deepEqual(
      remoteServerProbes([provider("ssh")], [{ isPending: true }]),
      {
        ssh: { status: "loading" },
      },
    );
  });

  it("reads a missing query slot as still in flight", () => {
    assert.deepEqual(remoteServerProbes([provider("ssh")], []), {
      ssh: { status: "loading" },
    });
  });

  it("carries a settled response through", () => {
    const result = { ok: true, name: "SSH", version: "0.4.26" };
    assert.deepEqual(
      remoteServerProbes(
        [provider("ssh")],
        [{ isPending: false, data: result }],
      ),
      { ssh: { status: "ok", result } },
    );
  });

  it("reports a thrown error's message", () => {
    assert.deepEqual(
      remoteServerProbes(
        [provider("ssh")],
        [{ isPending: false, error: new Error("spawn ENOENT") }],
      ),
      { ssh: { status: "failed", error: "spawn ENOENT" } },
    );
  });

  it("stringifies a non-Error rejection rather than dropping it", () => {
    assert.deepEqual(
      remoteServerProbes(
        [provider("ssh")],
        [{ isPending: false, error: "provider timed out" }],
      ),
      { ssh: { status: "failed", error: "provider timed out" } },
    );
  });

  it("fails a settled query that carries no response instead of spinning forever", () => {
    // A provider binary that prints bare `null` and exits 0 parses fine in
    // `invoke_provider` and its `ok` lookup is not `Some(false)`, so it reaches
    // here as a success with no body. Writing no entry would be read as
    // "still probing" by remoteServerEntries — a permanent spinner with no
    // error and no timeout.
    for (const data of [null, undefined]) {
      const probes = remoteServerProbes(
        [provider("ssh")],
        [{ data, isPending: false }],
      );
      assert.deepEqual(probes, {
        ssh: { status: "failed", error: PROVIDER_INFO_UNANSWERED },
      });
      assert.equal(
        remoteServerEntries([provider("ssh")], probes)[0].status,
        "unavailable",
      );
    }
  });

  it("keys each provider to its own query by position", () => {
    const probes = remoteServerProbes(
      [provider("ssh"), provider("blox")],
      [{ isPending: true }, { isPending: false, error: new Error("nope") }],
    );
    assert.equal(probes.ssh.status, "loading");
    assert.equal(probes.blox.status, "failed");
  });

  it("gives every provider an entry, so no row can be silently absent", () => {
    const providers = [provider("ssh"), provider("blox"), provider("zeta")];
    const probes = remoteServerProbes(providers, [
      { isPending: true },
      { data: { ok: true }, isPending: false },
      { data: null, isPending: false },
    ]);
    assert.deepEqual(Object.keys(probes).sort(), ["blox", "ssh", "zeta"]);
  });
});

describe("version reporting", () => {
  it("carries the probed version through for the row's pill", () => {
    const [entry] = remoteServerEntries([provider("ssh")], {
      ssh: okProbe(),
    });
    assert.equal(entry.version, "0.4.26");
  });

  it("is null when the provider reports no version", () => {
    const [entry] = remoteServerEntries([provider("ssh")], {
      ssh: okProbe({ version: undefined }),
    });
    assert.equal(entry.version, null);
    assert.equal(entry.status, "ready");
  });
});
