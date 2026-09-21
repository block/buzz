/**
 * Unit test for the PR6 hardcoded-string gate (spec NFR-007).
 *
 * Run from `desktop/`:
 *   node --import ./test-loader.mjs --experimental-strip-types --test scripts/check-hardcoded-strings.test.mjs
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  BASELINE_CATEGORIES,
  evaluateGate,
  parseBaseline,
  scanSourceText,
  shouldScanFile,
} from "./check-hardcoded-strings.mjs";

const REL = "src/features/messages/ui/Sample.tsx";

/** `kind=literal` for every candidate the scanner reports in `code`. */
const hits = (code, relPath = REL) =>
  scanSourceText(code, relPath).map((c) => `${c.kind}=${c.literal}`);

const candidate = (code, relPath = REL) => scanSourceText(code, relPath)[0];

test("flags a plain JSX text literal", () => {
  const code = `export const C = () => (
    <p>Send the message now</p>
  );`;
  assert.ok(hits(code).includes("jsx-text=Send the message now"));
});

test("flags a single-word JSX text literal (button labels)", () => {
  assert.deepEqual(hits(`export const C = () => <Button>Delete</Button>;`), [
    "jsx-text=Delete",
  ]);
});

test("flags an aria-label, in both the DOM and camelCase spelling", () => {
  assert.ok(
    hits(
      `export const C = () => <button aria-label="Close preview" />;`,
    ).includes("attr:aria-label=Close preview"),
  );
  assert.ok(
    hits(
      `export const C = () => <button ariaLabel="Close preview" />;`,
    ).includes("attr:ariaLabel=Close preview"),
  );
});

test("flags placeholder / title / label props and dialog + tooltip children", () => {
  const found = hits(`export const C = () => (
    <div>
      <input placeholder="Search people and agents" title="Copy link" />
      <Field label="Channel name" />
      <AlertDialogDescription>Delete the project for everyone.</AlertDialogDescription>
      <TooltipContent>Jump to the latest message</TooltipContent>
    </div>
  );`);
  for (const expected of [
    "attr:placeholder=Search people and agents",
    "attr:title=Copy link",
    "attr:label=Channel name",
    "jsx-text=Delete the project for everyone.",
    "jsx-text=Jump to the latest message",
  ])
    assert.ok(
      found.includes(expected),
      `missing ${expected}: ${found.join(" ")}`,
    );
});

test("flags string literals passed to toast.* and window.confirm", () => {
  const found = hits(`export const C = () => {
    toast.error("Could not save the profile");
    toast("Copied to clipboard");
    window.confirm("Delete this channel for everyone?");
    return null;
  };`);
  for (const expected of [
    "call:toast.error=Could not save the profile",
    "call:toast=Copied to clipboard",
    "call:confirm=Delete this channel for everyone?",
  ])
    assert.ok(
      found.includes(expected),
      `missing ${expected}: ${found.join(" ")}`,
    );
});

test("flags single-line concatenations and template literals in copy positions", () => {
  const found = hits(`export const C = ({ name, count }) => (
    <div title={"Delete " + name}>
      {\`Added \${count} agents\`}
    </div>
  );`);
  assert.ok(found.includes("attr:title=Delete {}"), found.join(" "));
  assert.ok(found.includes("jsx-expr=Added {} agents"), found.join(" "));
});

test("ignores console and logger calls", () => {
  assert.deepEqual(
    hits(`export const C = () => {
    console.warn("The relay connection dropped unexpectedly");
    logger.error("Something went wrong while loading the channel list");
    log.debug("Failed to parse the incoming event payload");
    return null;
  };`),
    [],
  );
});

test("ignores catalog keys, protocol identifiers, urls, colours and class names", () => {
  const found = hits(`export const C = ({ t }) => (
    <div className="flex items-center gap-2 text-sm" data-testid="composer-send">
      <p>{t("messages.composer.send")}</p>
      <p>Buzz</p>
      <p>Nostr</p>
      <input placeholder="https://buzz.tools/docs" />
      <span title="#0f172a">ok</span>
      <b>{"npub1l2vyh47mk2p0qlsku7hg0vn29faehy9hy34yga1pn067"}</b>
    </div>
  );
  const labelClassName = "text-sm leading-5";
  void labelClassName;`);
  assert.deepEqual(found, [], found.join(" "));
});

test("ignores test, spec, test-support, i18n and fixture files", () => {
  for (const skipped of [
    "src/features/messages/ui/MessageRow.test.tsx",
    "src/features/messages/ui/useMentionSendFlow.test-support.ts",
    "src/features/settings/__fixtures__/sampleData.ts",
    "src/features/settings/mocks/handlers.ts",
    "src/testing/e2eBridge.ts",
    "src/locales/en.json.ts",
    "src/i18n/index.ts",
    "src/app/routeTree.gen.ts",
    "src/types/global.d.ts",
  ])
    assert.equal(
      shouldScanFile(skipped),
      false,
      `${skipped} must not be scanned`,
    );

  for (const scanned of [
    "src/features/messages/ui/MessageRow.tsx",
    "src/shared/ui/dialog.tsx",
    "src/app/AppShell.tsx",
  ])
    assert.equal(shouldScanFile(scanned), true, `${scanned} must be scanned`);

  // A literal in a fixture file is never even parsed.
  assert.deepEqual(
    hits(
      `export const C = () => <p>Send the message now</p>;`,
      "src/features/settings/__fixtures__/sampleData.tsx",
    ).length,
    1,
    "scanSourceText is the extraction API; file filtering is shouldScanFile's job",
  );
});

test("a new candidate with no baseline entry fails the gate", () => {
  const c = candidate(`export const C = () => <p>Hello stranger</p>;`);
  assert.equal(c.literal, "Hello stranger");
  const result = evaluateGate({ candidates: [c], entries: [] });
  assert.equal(result.ok, false, "an unbaselined candidate must not pass");
  assert.deepEqual(
    result.unbaselined.map((u) => `${u.file}|${u.kind}|${u.literal}`),
    [`${REL}|jsx-text|Hello stranger`],
  );
});

test("a baselined candidate passes, and the same entry as true-violation fails", () => {
  const c = candidate(`export const C = () => <p>Hello stranger</p>;`);
  const base = { file: c.file, kind: c.kind, literal: c.literal };
  const allowed = evaluateGate({
    candidates: [c],
    entries: [
      {
        ...base,
        category: "deferred-wave-residual",
        note: "The owning wave has not extracted this file yet.",
      },
    ],
  });
  assert.equal(allowed.ok, true);
  assert.equal(allowed.violations.length, 0);

  const violated = evaluateGate({
    candidates: [c],
    entries: [
      {
        ...base,
        category: "true-violation",
        note: "Untranslated UI copy left behind by a merged wave.",
      },
    ],
  });
  assert.equal(violated.ok, false, "a true-violation entry must fail the gate");
  assert.equal(violated.violations.length, 1);

  // A candidate the baseline never mentions still fails even when the baseline
  // is otherwise populated (no implicit pass for unknown files).
  const other = evaluateGate({
    candidates: [c],
    entries: [
      {
        file: "src/features/messages/ui/Elsewhere.tsx",
        kind: "jsx-text",
        literal: "Hello stranger",
        category: "deferred-pr4-surface",
        note: "Another file's entry cannot cover this one.",
      },
    ],
  });
  assert.equal(other.ok, false);
  assert.equal(other.unbaselined.length, 1);
});

test("the baseline document is validated against the fixed vocabulary", () => {
  const group = (override) => ({
    file: REL,
    category: "brand-name",
    note: "Group reason.",
    items: [{ kind: "jsx-text", literal: "Hello stranger" }],
    ...override,
  });
  assert.ok(Object.hasOwn(BASELINE_CATEGORIES, "deferred-pr4-surface"));
  assert.ok(Object.hasOwn(BASELINE_CATEGORIES, "true-violation"));

  // Only a surface deferral may share the group note.
  assert.deepEqual(parseBaseline({ entries: [group()] }).errors, [
    `${"group #1"} item #1: category \`brand-name\` requires its own note on the literal`,
  ]);
  assert.deepEqual(
    parseBaseline({
      entries: [group({ category: "no-such-reason", note: "x" })],
    }).errors[0],
    "group #1: unknown category `no-such-reason`; allowed: " +
      Object.keys(BASELINE_CATEGORIES).join(", "),
  );
  assert.deepEqual(parseBaseline({ entries: [] }).entries, []);
  assert.match(parseBaseline({}).errors[0], /no `entries` array/);
  assert.deepEqual(
    parseBaseline({
      entries: [
        {
          file: REL,
          category: "deferred-pr4-surface",
          note: "Surface reason.",
          items: [{ kind: "jsx-text", literal: "Hello stranger" }],
        },
      ],
    }).entries,
    [
      {
        file: REL,
        kind: "jsx-text",
        literal: "Hello stranger",
        category: "deferred-pr4-surface",
        note: "Surface reason.",
      },
    ],
  );
});
