import assert from "node:assert/strict";
import test from "node:test";

import {
  matchLeadingMention,
  renderHighlightedContent,
} from "./highlightContent.tsx";

const channels = [
  { id: "c1", name: "fix login bug" },
  { id: "c2", name: "fix login bug in prod" },
  { id: "c3", name: "deploys" },
];

function channelRefOpts(onOpen = () => {}) {
  return { channels, onOpen };
}

function buttons(nodes) {
  return nodes.filter(
    (node) =>
      typeof node === "object" && node !== null && node.type === "button",
  );
}

test("knownChannelRef_rendersClickableButton", () => {
  let opened = null;
  const nodes = renderHighlightedContent(
    "see #deploys for status",
    [],
    channelRefOpts((id) => {
      opened = id;
    }),
  );
  const [button] = buttons(nodes);
  assert.ok(button, "expected a button node for the channel ref");
  assert.equal(button.props.children, "#deploys");
  button.props.onClick();
  assert.equal(opened, "c3");
});

test("unknownChannelRef_staysPlainText", () => {
  const nodes = renderHighlightedContent(
    "priority #1 for today",
    [],
    channelRefOpts(),
  );
  assert.equal(buttons(nodes).length, 0);
  assert.ok(nodes.join("").includes("#1"));
});

test("channelRef_withoutOptions_staysPlainText", () => {
  const nodes = renderHighlightedContent("see #deploys");
  assert.equal(buttons(nodes).length, 0);
});

test("channelRef_matchesLongestNameWithSpaces", () => {
  const nodes = renderHighlightedContent(
    "tracking #fix login bug in prod now",
    [],
    channelRefOpts(),
  );
  const [button] = buttons(nodes);
  assert.ok(button);
  assert.equal(button.props.children, "#fix login bug in prod");
  // Remaining prose after the ref is preserved.
  const tail = nodes[nodes.indexOf(button) + 1];
  assert.equal(tail, " now");
});

test("channelRef_matchIsCaseInsensitive", () => {
  const nodes = renderHighlightedContent("see #Deploys", [], channelRefOpts());
  const [button] = buttons(nodes);
  assert.ok(button);
  assert.equal(button.props.children, "#Deploys");
});

test("channelRef_requiresBoundaryAfterName", () => {
  // "#deploysX" is not the known channel "deploys".
  const nodes = renderHighlightedContent("see #deploysX", [], channelRefOpts());
  assert.equal(buttons(nodes).length, 0);
});

const ampMention = { name: "amp (local)", color: "#00bcd4" };

test("leadingMention_knownName_matchesPastTrailingSpace", () => {
  const directed = matchLeadingMention("@amp (local) open a PR", [ampMention]);
  assert.ok(directed);
  assert.equal(directed.mention.name, "amp (local)");
  assert.equal("@amp (local) open a PR".slice(directed.end), "open a PR");
});

test("leadingMention_matchesEntireMessage", () => {
  const directed = matchLeadingMention("@amp (local)", [ampMention]);
  assert.ok(directed);
  assert.equal("@amp (local)".slice(directed.end), "");
});

test("leadingMention_midMessageMention_doesNotMatch", () => {
  assert.equal(
    matchLeadingMention("ask @amp (local) later", [ampMention]),
    null,
  );
});

test("leadingMention_unknownName_doesNotMatch", () => {
  assert.equal(matchLeadingMention("@stranger hello", [ampMention]), null);
});

test("leadingMention_requiresBoundaryAfterName", () => {
  // "@amp (local)x" is not the known mention "amp (local)".
  assert.equal(matchLeadingMention("@amp (local)x hi", [ampMention]), null);
});
