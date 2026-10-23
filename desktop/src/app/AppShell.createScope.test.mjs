import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, test } from "node:test";
import * as React from "react";
import { createRoot } from "react-dom/client";
import {
  QueryClient,
  QueryClientProvider,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import ts from "typescript";

const dom = new JSDOM("<!doctype html><html><body></body></html>");
Object.assign(globalThis, {
  document: dom.window.document,
  window: dom.window,
  IS_REACT_ACT_ENVIRONMENT: true,
});
after(() => dom.window.close());

// Bind the actual callbacks and mutation hook without mounting the unrelated
// desktop surfaces. Native I/O is deferred; React's lifecycle and QueryClient
// are real. Removing a guard from AppShell must break these tests.
const source = ts.createSourceFile(
  "AppShell.tsx",
  readFileSync(new URL("./AppShell.tsx", import.meta.url), "utf8"),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const shell = source.statements.find(
  (node) => ts.isFunctionDeclaration(node) && node.name?.text === "AppShell",
);
const wanted = new Set([
  "createScopeRef",
  "handleCreateChannel",
  "handleCreateForum",
]);
const statements = shell.body.statements
  .filter(
    (node) =>
      (ts.isVariableStatement(node) &&
        node.declarationList.declarations.some((decl) =>
          wanted.has(decl.name.getText(source)),
        )) ||
      (ts.isExpressionStatement(node) &&
        node.expression.getText(source).startsWith("React.useLayoutEffect(")),
  )
  .map((node) => node.getText(source));
const hooks = ts.createSourceFile(
  "hooks.ts",
  readFileSync(
    new URL("../features/channels/hooks.ts", import.meta.url),
    "utf8",
  ),
  ts.ScriptTarget.Latest,
  true,
);
const mutation = hooks.statements
  .find(
    (node) =>
      ts.isFunctionDeclaration(node) &&
      node.name?.text === "useCreateChannelMutation",
  )
  .getText(hooks)
  .replace(/^export /, "");
const compile = (text) =>
  ts.transpileModule(text, {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.None,
    },
  }).outputText;
const makeHarness = new Function(
  "React",
  "useMutation",
  "useQueryClient",
  "createChannel",
  compile(`
const channelsQueryKey = ["channels"];
const upsertCachedChannel = (current = [], channel) => [...current, channel];
${mutation}
return function Harness({ community, sinks, ready }) {
  const communitiesHook = { activeCommunity: community };
  const createChannelMutation = useCreateChannelMutation();
  const createForumMutation = useCreateChannelMutation();
  const { applyCanvas, goChannel, applyAgents } = sinks;
  ${statements.join("\n")}
  React.useLayoutEffect(() => { ready({ stream: handleCreateChannel, forum: handleCreateForum }); });
  return null;
};
`),
);

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

for (const type of ["stream", "forum"]) {
  for (const phase of ["create", "canvas", "navigation"]) {
    for (const transition of ["unmount", "rerender", "roundtrip", "none"]) {
      test(`${type}: ${transition} during ${phase} discards remaining side effects`, async () => {
        const pending = deferred();
        const calls = [];
        const created = { id: "community-a-channel" };
        const Harness = makeHarness(
          React,
          useMutation,
          useQueryClient,
          async (input) => {
            assert.equal(input.channelType, type);
            calls.push("create");
            if (phase === "create") await pending.promise;
            return created;
          },
        );
        const sinks = {
          applyCanvas: async () => {
            calls.push("canvas");
            if (phase === "canvas") await pending.promise;
          },
          goChannel: async () => {
            calls.push("navigation");
            if (phase === "navigation") await pending.promise;
          },
          applyAgents: () => calls.push("agents"),
        };
        const client = new QueryClient({
          defaultOptions: { mutations: { retry: false, gcTime: 0 } },
        });
        const container = document.createElement("div");
        const root = createRoot(container);
        let handlers;
        let completion;
        let unmounted = false;
        const render = (id) =>
          React.createElement(
            QueryClientProvider,
            { client },
            React.createElement(Harness, {
              // Same signing identity in both communities: identity comparison alone
              // cannot protect the relay boundary.
              community: {
                id,
                relayUrl: `https://${id}.example`,
                pubkey: "same-signer",
              },
              sinks,
              ready: (value) => {
                handlers = value;
              },
            }),
          );
        try {
          await React.act(async () => {
            root.render(render("a"));
          });
          await React.act(async () => {
            completion = handlers[type](
              { name: "test", visibility: "open", templateId: "template" },
              () => calls.push("onCreated"),
            );
            await new Promise((resolve) => setImmediate(resolve));
          });
          assert.equal(calls.at(-1), phase);
          await React.act(async () => {
            if (transition === "unmount") {
              root.unmount();
              unmounted = true;
            } else if (transition !== "none") root.render(render("b"));
          });
          if (transition === "roundtrip") {
            await React.act(async () => {
              root.render(render("a"));
            });
          }
          const before = [...calls];
          await React.act(async () => {
            pending.resolve();
            await completion;
          });
          assert.deepEqual(
            calls,
            transition === "none"
              ? [
                  "create",
                  "canvas",
                  "navigation",
                  ...(type === "stream" ? ["onCreated"] : []),
                  "agents",
                ]
              : before,
          );
        } finally {
          pending.resolve();
          if (!unmounted)
            await React.act(async () => {
              root.unmount();
            });
          client.clear();
          container.remove();
        }
      });
    }
  }
}
