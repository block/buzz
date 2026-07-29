import assert from "node:assert/strict";
import { describe, it } from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  ChannelMenuButton,
  ChannelWorkingIndicator,
  formatWorkingTooltip,
} from "./SidebarSection.tsx";
import { SidebarProvider } from "../../../shared/ui/sidebar.tsx";

function summary(agentNames, agentCount = agentNames.length) {
  return {
    channelId: "chan-1",
    anchorAt: 0,
    agentCount,
    agentPubkeys: Array.from(
      { length: agentCount },
      (_, index) => `agent-${index}-pubkey`,
    ),
    agentNames,
  };
}

function channel(name, channelType, visibility = "open") {
  return {
    id: `${channelType}-${visibility}-${name}`,
    name,
    channelType,
    visibility,
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys:
      channelType === "dm" ? ["viewer-pubkey", "agent-pubkey"] : [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

describe("formatWorkingTooltip", () => {
  it("names one known agent", () => {
    assert.equal(formatWorkingTooltip(summary(["Ned"])), "Ned working");
  });

  it("names one known agent and counts one additional agent", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart"])),
      "Ned and 1 agent working",
    );
  });

  it("names one known agent and counts multiple additional agents", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl"])),
      "Ned and 2 agents working",
    );
  });

  it("uses a singular count when all agents are unknown", () => {
    assert.equal(formatWorkingTooltip(summary([], 1)), "1 agent working");
  });

  it("uses a plural count when all agents are unknown", () => {
    assert.equal(formatWorkingTooltip(summary([], 3)), "3 agents working");
  });

  it("counts unknown agents with the named lead", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned"], 3)),
      "Ned and 2 agents working",
    );
  });
});

describe("ChannelWorkingIndicator", () => {
  it("renders a subtle spinner instead of an elapsed-time counter", () => {
    const html = renderToStaticMarkup(
      React.createElement(ChannelWorkingIndicator, {
        channelName: "agent-work",
        isActive: false,
        summary: summary(["Ned"]),
      }),
    );

    assert.match(html, /lucide-loader-circle/);
    assert.match(html, /motion-safe:animate-spin/);
    assert.match(html, /text-sidebar-foreground\/45/);
    assert.match(html, /aria-label="Ned working"/);
    assert.doesNotMatch(html, /tabular-nums/);
    assert.doesNotMatch(html, />0s</);
  });
});

describe("ChannelMenuButton", () => {
  it("uses the spinner for every supported left-nav channel item type", () => {
    const itemTypes = [
      channel("general", "stream"),
      channel("private-team", "stream", "private"),
      channel("help-forum", "forum"),
      channel("agent-dm", "dm", "private"),
    ];

    for (const item of itemTypes) {
      const html = renderToStaticMarkup(
        React.createElement(
          SidebarProvider,
          null,
          React.createElement(ChannelMenuButton, {
            activeWorking: summary(["Ned"]),
            channel: item,
            hasUnread: false,
            isActive: false,
            onSelectChannel() {},
          }),
        ),
      );

      assert.match(html, /lucide-loader-circle/, item.name);
      assert.match(html, /motion-safe:animate-spin/, item.name);
      assert.doesNotMatch(html, /tabular-nums/, item.name);
      assert.doesNotMatch(html, />0s</, item.name);
    }
  });
});
