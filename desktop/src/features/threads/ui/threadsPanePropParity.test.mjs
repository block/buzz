/**
 * Threads pane ↔ channel thread panel parity guard.
 *
 * The Threads view shows threads through ProjectConversationPanel, which
 * renders the same MessageThreadPanel the channel uses. Each panel feature is
 * a prop the surrounding screen has to pass, so a prop added upstream silently
 * stays missing from the Threads pane. This test lists every MessageThreadPanel
 * prop and fails until each one is either passed by ProjectConversationPanel
 * or recorded below as deliberately left out, with the reason.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

/** Props the Threads pane deliberately does not pass. */
const DELIBERATELY_OMITTED = {
  autoSendDraftKey:
    "Send-from-drafts navigates to the channel route, which owns the auto-submit.",
  onAutoSubmitComplete: "Pairs with autoSendDraftKey.",
  huddleMemberPubkeys: "Huddle transcripts open in their channel only.",
  huddleMemberPubkeysPending: "Huddle transcripts open in their channel only.",
  isHuddleTranscript: "Huddle transcripts open in their channel only.",
  searchMessageId:
    "The pane is opened from the Threads list, never from a search hit.",
  searchQuery: "Pairs with searchMessageId.",
  videoReviewPresentation: "Video review threads open in their channel only.",
};

function parse(relativePath) {
  const url = new URL(relativePath, import.meta.url);
  return ts.createSourceFile(
    url.pathname,
    readFileSync(url, "utf8"),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
}

function findAll(node, predicate, found = []) {
  if (predicate(node)) found.push(node);
  // forEachChild stops at the first truthy callback result, so return nothing.
  ts.forEachChild(node, (child) => {
    findAll(child, predicate, found);
  });
  return found;
}

function typeAlias(source, name) {
  const [alias] = findAll(
    source,
    (node) => ts.isTypeAliasDeclaration(node) && node.name.text === name,
  );
  assert.ok(alias, `type ${name} not found in ${source.fileName}`);
  return alias;
}

function memberNames(typeLiteral) {
  return typeLiteral.members
    .filter((member) => member.name)
    .map((member) => member.name.text);
}

function propertyKeys(node) {
  return findAll(
    node,
    (child) =>
      (ts.isPropertyAssignment(child) ||
        ts.isShorthandPropertyAssignment(child)) &&
      ts.isIdentifier(child.name),
  ).map((child) => child.name.text);
}

const panelSource = parse("../../messages/ui/MessageThreadPanel.tsx");
const layoutSource = parse("../../channels/lib/threadPanelLayout.ts");
const paneSource = parse("../../projects/ui/ProjectConversationPanel.tsx");

/** Every prop MessageThreadPanel accepts, layout props included. */
function panelProps() {
  const props = new Set();
  const visit = (typeNode) => {
    if (ts.isIntersectionTypeNode(typeNode)) {
      for (const part of typeNode.types) visit(part);
    } else if (ts.isTypeLiteralNode(typeNode)) {
      for (const name of memberNames(typeNode)) props.add(name);
    } else if (
      ts.isTypeReferenceNode(typeNode) &&
      typeNode.typeName.getText() === "ThreadPanelLayoutProps"
    ) {
      visit(typeAlias(layoutSource, "ThreadPanelLayoutProps").type);
    } else {
      assert.fail(`Unhandled prop type part: ${typeNode.getText()}`);
    }
  };
  visit(typeAlias(panelSource, "MessageThreadPanelProps").type);
  return props;
}

/** Props the Threads pane's <MessageThreadPanel> receives, spreads included. */
function paneProps() {
  const [element] = findAll(
    paneSource,
    (node) =>
      (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) &&
      node.tagName.getText() === "MessageThreadPanel",
  );
  assert.ok(element, "ProjectConversationPanel renders no MessageThreadPanel");
  const props = new Set();
  for (const attribute of element.attributes.properties) {
    if (ts.isJsxAttribute(attribute)) {
      props.add(attribute.name.getText());
      continue;
    }
    const spread = attribute.expression.getText();
    const [declaration] = findAll(
      paneSource,
      (node) =>
        ts.isVariableDeclaration(node) &&
        node.name.getText() === spread &&
        node.initializer,
    );
    assert.ok(declaration, `cannot resolve spread {...${spread}}`);
    for (const key of propertyKeys(declaration.initializer)) props.add(key);
    // The focus drawer layout comes from the shared helper's return value.
    const callsLayoutHelper = findAll(
      declaration.initializer,
      (node) =>
        ts.isCallExpression(node) &&
        node.expression.getText() === "getThreadPanelLayout",
    ).length;
    if (callsLayoutHelper) {
      const [helper] = findAll(
        layoutSource,
        (node) =>
          ts.isFunctionDeclaration(node) &&
          node.name?.text === "getThreadPanelLayout",
      );
      for (const literal of findAll(
        helper.body,
        ts.isObjectLiteralExpression,
      )) {
        for (const key of propertyKeys(literal)) props.add(key);
      }
    }
  }
  return props;
}

const allProps = panelProps();
const wiredProps = paneProps();

test("the prop inventory parses (guards against an empty, vacuous pass)", () => {
  assert.ok(allProps.size > 40, `only ${allProps.size} props found`);
  for (const known of ["firstUnreadReplyId", "onSend", "isFocusMode"]) {
    assert.ok(allProps.has(known), `${known} missing from inventory`);
  }
});

test("every MessageThreadPanel prop is wired in the Threads pane or deliberately omitted", () => {
  const unhandled = [...allProps].filter(
    (prop) => !wiredProps.has(prop) && !(prop in DELIBERATELY_OMITTED),
  );
  assert.deepEqual(
    unhandled,
    [],
    "MessageThreadPanel gained props the Threads pane does not pass. Wire " +
      "each in ProjectConversationPanel, or add it to DELIBERATELY_OMITTED " +
      "with the reason.",
  );
});

test("the omission list has no stale entries", () => {
  for (const prop of Object.keys(DELIBERATELY_OMITTED)) {
    assert.ok(allProps.has(prop), `${prop} is no longer a panel prop`);
    assert.ok(!wiredProps.has(prop), `${prop} is wired now; drop the omission`);
  }
});
