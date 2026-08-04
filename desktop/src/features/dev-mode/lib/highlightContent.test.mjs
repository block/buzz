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

function findByType(nodes, type) {
  return nodes.find(
    (node) => typeof node === "object" && node !== null && node.type === type,
  );
}

test("boldMarkdown_rendersStrongWithoutMarkers", () => {
  const nodes = renderHighlightedContent("**Verified (independently)** — ok");
  const strong = findByType(nodes, "strong");
  assert.ok(strong);
  assert.deepEqual(strong.props.children, ["Verified (independently)"]);
  assert.ok(nodes.includes(" — ok"));
});

test("italicMarkdown_rendersEm", () => {
  const star = findByType(
    renderHighlightedContent("really *important* here"),
    "em",
  );
  assert.deepEqual(star.props.children, ["important"]);
  const underscore = findByType(
    renderHighlightedContent("really _important_ here"),
    "em",
  );
  assert.deepEqual(underscore.props.children, ["important"]);
});

test("strikeMarkdown_rendersDel", () => {
  const del = findByType(
    renderHighlightedContent("~~old plan~~ new plan"),
    "del",
  );
  assert.deepEqual(del.props.children, ["old plan"]);
});

test("boldContainingInlineCode_nestsHighlighting", () => {
  const strong = findByType(
    renderHighlightedContent("**uses `flag`**"),
    "strong",
  );
  assert.ok(strong);
  const [prefix, code] = strong.props.children;
  assert.equal(prefix, "uses ");
  assert.equal(code.props.children, "flag");
});

test("snakeCaseAndArithmetic_stayPlainProse", () => {
  for (const text of ["field_for_json stays", "2*3*4 = 24", "a ** b"]) {
    const nodes = renderHighlightedContent(text);
    assert.equal(findByType(nodes, "em"), undefined, text);
    assert.equal(findByType(nodes, "strong"), undefined, text);
  }
});

test("markdownLink_rendersDevLinkWithLabel", () => {
  const nodes = renderHighlightedContent(
    "see [the PR](https://github.com/x/y/pull/1).",
  );
  const link = nodes.find(
    (node) => typeof node === "object" && node !== null && node.props?.href,
  );
  assert.ok(link);
  assert.equal(link.props.href, "https://github.com/x/y/pull/1");
  assert.equal(link.props.label, "the PR");
  assert.equal(nodes.at(-1), ".");
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
