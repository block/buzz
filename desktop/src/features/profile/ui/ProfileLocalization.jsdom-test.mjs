import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, render } from "@testing-library/react";
import { setAppLanguage } from "../../../i18n.ts";
import { uiT, useUiLanguage } from "../../../shared/i18n/uiTranslation.ts";
import {
  buildPublicFields,
  buildOwnerFields,
  bucketProfileFields,
  ProfileFieldRows,
} from "./UserProfilePanelFields.tsx";
import { NostrBindButtonLabel } from "./NostrBindButtonLabel.tsx";

afterEach(async () => {
  cleanup();
  await setAppLanguage("en");
  localStorage.clear();
});

test("profile builders retain fields and literal values across languages", async () => {
  const pubkey = "a".repeat(64);
  const relayAgent = {
    agentType: "codex-acp",
    capabilities: ["Settings {{p1}}"],
    respondTo: "owner-only",
  };
  const managedAgent = {
    agentCommand: "codex-acp",
    acpCommand: "raw {{p1}}",
    mcpCommand: "raw-mcp",
    backend: { type: "provider", id: "local" },
    startOnAppLaunch: true,
    status: "stopped",
    lastError: "raw error {{p0}}",
  };
  let original;
  for (const language of ["en", "ko", "en"]) {
    await setAppLanguage(language);
    const fields = [
      ...buildPublicFields({
        isBot: true,
        pubkey,
        relayAgent,
        profile: undefined,
      }),
      ...buildOwnerFields({
        includeOperationalFields: true,
        managedAgent,
        relayAgent,
        ownerDisplayName: "Alice {{p1}}",
        ownerHandle: null,
        ownerProfilePubkey: null,
        ownerPubkey: pubkey,
        presenceLoaded: true,
        presenceStatus: "offline",
      }),
    ];
    const buckets = bucketProfileFields(fields);
    const ids = Object.fromEntries(
      Object.entries(buckets).map(([k, v]) => [k, v.map((f) => f.id)]),
    );
    if (original) assert.deepEqual(ids, original);
    else original = ids;
    assert.ok(ids.agentInfoFields.includes("public-key"));
    assert.ok(ids.agentInfoFields.includes("capabilities"));
    assert.ok(ids.agentSettingsFields.includes("start-on-launch"));
    assert.ok(ids.diagnosticsFields.includes("last-error"));
    assert.equal(
      fields.find((f) => f.id === "capabilities").copyValue,
      "Settings {{p1}}",
    );
    assert.equal(
      fields.find((f) => f.id === "last-error").displayValue,
      "raw error {{p0}}",
    );
    const screen = render(React.createElement(ProfileFieldRows, { fields }));
    assert.ok(
      screen.container.textContent.includes(
        language === "ko" ? "공개 키" : "Public key",
      ),
    );
    assert.equal(
      screen.container.querySelectorAll(
        '[data-testid="user-profile-start-on-launch"]',
      ).length,
      1,
    );
    screen.unmount();
  }
});

test("binding action has exactly one visible label in each state and language", async () => {
  for (const [idle, active] of [
    ["Continue", "Signing…"],
    ["Copy response", "Copied"],
  ]) {
    function Screen({ busy }) {
      useUiLanguage();
      return React.createElement(NostrBindButtonLabel, {
        active: busy,
        idleLabel: uiT(idle),
        activeLabel: uiT(active),
      });
    }
    const screen = render(React.createElement(Screen, { busy: false }));
    for (const language of ["en", "ko", "en"]) {
      await act(() => setAppLanguage(language));
      for (const busy of [false, true]) {
        screen.rerender(React.createElement(Screen, { busy }));
        const visible = screen.container.querySelectorAll(".opacity-100");
        assert.equal(visible.length, 1);
        assert.equal(visible[0].textContent, uiT(busy ? active : idle));
      }
    }
    screen.unmount();
  }
});
