import assert from "node:assert/strict";
import test from "node:test";

import { parseChannelMcpAppStore } from "./channelMcpAppStorage.ts";

test("parses only complete MCP App installations", () => {
  assert.deepEqual(
    parseChannelMcpAppStore({
      version: 1,
      channels: {
        channel: [
          {
            id: "board",
            endpoint: "https://runtime.example/mcp",
            serverName: "Runtime",
            toolName: "board.open",
            title: "Board",
            resourceUri: "ui://runtime/board",
            arguments: { space: "alpha" },
            approvedPolicy: {
              csp: {
                connectDomains: ["https://api.example.com"],
              },
              requestedPermissions: {},
            },
          },
          {
            id: "bad",
            endpoint: "https://runtime.example/mcp",
            toolName: "bad.open",
            resourceUri: "https://runtime.example/app",
          },
        ],
      },
    }),
    {
      version: 1,
      channels: {
        channel: [
          {
            id: "board",
            endpoint: "https://runtime.example/mcp",
            serverName: "Runtime",
            toolName: "board.open",
            title: "Board",
            resourceUri: "ui://runtime/board",
            arguments: { space: "alpha" },
            approvedPolicy: {
              csp: {
                connectDomains: ["https://api.example.com"],
                resourceDomains: [],
                frameDomains: [],
                baseUriDomains: [],
              },
              requestedPermissions: {
                camera: undefined,
                microphone: undefined,
                geolocation: undefined,
                clipboardWrite: undefined,
              },
            },
          },
        ],
      },
    },
  );
});

test("sanitizes untrusted display labels and defaults legacy policy closed", () => {
  const store = parseChannelMcpAppStore({
    version: 1,
    channels: {
      channel: [
        {
          id: "board",
          endpoint: "https://runtime.example/mcp",
          serverName: "Buzz\u202e Security",
          toolName: "board.open",
          title: `${"A".repeat(90)}\u0000`,
          resourceUri: "ui://runtime/board",
          arguments: {},
        },
      ],
    },
  });
  const app = store.channels.channel[0];
  assert.equal(app.serverName, "Buzz Security");
  assert.equal(app.title.length, 80);
  assert.deepEqual(app.approvedPolicy.csp.connectDomains, []);
});

test("rejects unknown store versions", () => {
  assert.equal(parseChannelMcpAppStore({ version: 2, channels: {} }), null);
});
