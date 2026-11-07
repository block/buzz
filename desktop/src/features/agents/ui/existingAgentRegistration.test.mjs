import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  existingAgentRegistrationMessage,
  RegisterExistingAgentPane,
} from "./ExistingAgentRegistrationPane.tsx";

test("registration pane states the ownership, discovery, and private-key boundaries", () => {
  const html = renderToStaticMarkup(
    React.createElement(RegisterExistingAgentPane, {
      isPending: false,
      onRegister: async () => {
        throw new Error("unused");
      },
    }),
  );

  assert.match(html, /only you can find this agent in mention suggestions/i);
  assert.match(html, /does not[\s\S]*store the agent.*private key/i);
  assert.match(html, /published profile must prove that you are the owner/i);
  assert.match(html, /controls Desktop discovery only/i);
  assert.match(html, /external agent runtime to accept the same people/i);
  assert.match(html, /register-existing-agent-respond-to/);
  assert.match(html, /register-existing-agent-submit/);
  assert.match(html, /disabled/);
});

test("registration result copy distinguishes published, queued, and existing", () => {
  const base = {
    agentPubkey: "a".repeat(64),
    displayName: "Tess",
    relayMessage: null,
  };
  assert.match(
    existingAgentRegistrationMessage({
      ...base,
      publicationStatus: "published",
      alreadyRegistered: false,
    }),
    /registered and can appear in mention suggestions/,
  );
  assert.match(
    existingAgentRegistrationMessage({
      ...base,
      publicationStatus: "queued",
      alreadyRegistered: false,
    }),
    /queued and will publish/,
  );
  assert.equal(
    existingAgentRegistrationMessage({
      ...base,
      publicationStatus: "published",
      alreadyRegistered: true,
    }),
    "Tess is already registered.",
  );
});
