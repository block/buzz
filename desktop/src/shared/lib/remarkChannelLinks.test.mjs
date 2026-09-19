import assert from "node:assert/strict";
import test from "node:test";
import { fromMarkdown } from "mdast-util-from-markdown";

import remarkChannelLinks from "./remarkChannelLinks.ts";
import remarkMentions from "./remarkMentions.ts";

function parse(content, channelNames) {
  const tree = fromMarkdown(content);
  remarkChannelLinks({ channelNames })(tree);
  return tree;
}

function nodesOfType(tree, type) {
  return [
    ...(tree.type === type ? [tree] : []),
    ...(tree.children ?? []).flatMap((child) => nodesOfType(child, type)),
  ];
}

function chips(tree) {
  return nodesOfType(tree, "channel-link").map((node) => node.value);
}

for (const names of [[], ["general"], ["general", "getting-started"]]) {
  test(`channel chips survive loading and unrelated membership: ${JSON.stringify(names)}`, () => {
    const tree = parse("See #general and #getting-started.", names);
    assert.deepEqual(chips(tree), ["#general", "#getting-started"]);
    const chip = nodesOfType(tree, "channel-link")[1];
    assert.equal(chip.data.hName, "channel-link");
    assert.equal(chip.data.channelName, "getting-started");
    assert.deepEqual(chip.data.hChildren, [
      { type: "text", value: "#getting-started" },
    ]);
    assert.equal(tree.children[0].children.at(-1).value, ".");
  });

  test(`generic tokens exclude punctuation and URL fragments: ${JSON.stringify(names)}`, () => {
    assert.deepEqual(
      chips(parse("(#getting-started), #team_2! #123? #new-room;", names)),
      ["#getting-started", "#team_2", "#123", "#new-room"],
    );
    assert.deepEqual(
      chips(
        parse(
          "https://example.com/a#getting-started word#getting-started /#general :#general -#general .#general",
          names,
        ),
      ),
      [],
    );
  });

  test(`code and authored links are not rewritten: ${JSON.stringify(names)}`, () => {
    const tree = parse(
      "`#getting-started` [#getting-started](https://example.com)\n\n```text\n#getting-started\n```",
      names,
    );
    assert.deepEqual(chips(tree), []);
    assert.equal(nodesOfType(tree, "inlineCode")[0].value, "#getting-started");
    assert.equal(nodesOfType(tree, "code")[0].value, "#getting-started");
    assert.equal(
      nodesOfType(tree, "link")[0].children[0].value,
      "#getting-started",
    );
  });
}

test("known names remain longest-first, case-insensitive, and regex-escaped", () => {
  assert.deepEqual(
    chips(
      parse("#Design Team and #Design; #C++ plus #GETTING-STARTED", [
        "Design",
        "Design Team",
        "C++",
      ]),
    ),
    ["#Design Team", "#Design", "#C++", "#GETTING-STARTED"],
  );
});

test("a known prefix does not truncate a longer unknown channel", () => {
  assert.deepEqual(chips(parse("#general-help", ["general"])), [
    "#general-help",
  ]);
});

test("channel fallback does not loosen user mentions", () => {
  for (const mentionNames of [[], ["Alice Smith"]]) {
    const tree = parse("@unknown @Alice Smith #getting-started", ["general"]);
    remarkMentions({ mentionNames })(tree);
    assert.deepEqual(
      nodesOfType(tree, "mention").map((node) => node.value),
      mentionNames.length ? ["@Alice Smith"] : [],
    );
    assert.deepEqual(chips(tree), ["#getting-started"]);
  }
});
