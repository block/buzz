import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { NO_BACKEND_PROVIDER_HINT } from "../../agents/lib/backendProviderLabel.ts";
import { remoteRunNotice } from "./remoteRunNotice.ts";

function provider(id) {
  return { id, binaryPath: `/usr/local/bin/buzz-backend-${id}` };
}

describe("remoteRunNotice", () => {
  it("says nothing while discovery is in flight", () => {
    assert.deepEqual(
      remoteRunNotice({ isLoading: true, providers: undefined }),
      { kind: "pending" },
    );
  });

  it("stays pending mid-flight even if a stale list is cached", () => {
    // Rendering the install hint over a cached non-empty list, or vice versa,
    // would contradict itself a frame later.
    assert.equal(
      remoteRunNotice({ isLoading: true, providers: [provider("ssh")] }).kind,
      "pending",
    );
  });

  it("shows the install hint when no provider is installed", () => {
    const notice = remoteRunNotice({ isLoading: false, providers: [] });
    assert.equal(notice.kind, "hint");
    assert.equal(notice.message, NO_BACKEND_PROVIDER_HINT);
  });

  it("treats an undefined result as none discovered", () => {
    assert.equal(
      remoteRunNotice({ isLoading: false, providers: undefined }).kind,
      "hint",
    );
  });

  it("names the discovered provider and points at the create flow", () => {
    const notice = remoteRunNotice({
      isLoading: false,
      providers: [provider("ssh")],
    });
    assert.equal(notice.kind, "ready");
    assert.equal(
      notice.message,
      "ssh detected — pick a server when you create an agent.",
    );
  });

  it("lists several providers in a stable order", () => {
    const notice = remoteRunNotice({
      isLoading: false,
      providers: [provider("ssh"), provider("blox")],
    });
    assert.equal(
      notice.message,
      "blox, ssh detected — pick a server when you create an agent.",
    );
  });
});
