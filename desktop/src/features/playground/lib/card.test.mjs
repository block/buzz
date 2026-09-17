import assert from "node:assert/strict";
import test from "node:test";

import { extractPlaygroundCard, parsePlaygroundCard } from "./card.ts";

const valid = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "1234",
  sid: "demo-1",
};

test("parses required playground card fields without rewriting them", () => {
  const card = parsePlaygroundCard(JSON.stringify(valid));
  assert.deepEqual(card, valid);
});

test("keeps optional stack and expires only when present", () => {
  const card = parsePlaygroundCard(
    JSON.stringify({ ...valid, stack: "hula-app", expires: "2026-08-22" }),
  );
  assert.equal(card?.stack, "hula-app");
  assert.equal(card?.expires, "2026-08-22");
  assert.equal("stack" in parsePlaygroundCard(JSON.stringify(valid)), false);
});

test("rejects malformed cards and non-https urls", () => {
  assert.equal(parsePlaygroundCard("{"), null);
  assert.equal(
    parsePlaygroundCard(JSON.stringify({ ...valid, hula: "pin" })),
    null,
  );
  assert.equal(parsePlaygroundCard(JSON.stringify({ ...valid, v: 2 })), null);
  assert.equal(
    parsePlaygroundCard(JSON.stringify({ ...valid, name: "" })),
    null,
  );
  assert.equal(
    parsePlaygroundCard(
      JSON.stringify({ ...valid, url: "http://app.example.com" }),
    ),
    null,
  );
  assert.equal(
    parsePlaygroundCard(
      JSON.stringify({ ...valid, url: "https://app.example.com:18789" }),
    ),
    null,
  );
});

test("accepts missing or empty pin and stays openable", () => {
  const withoutPin = { ...valid };
  delete withoutPin.pin;
  const omitted = parsePlaygroundCard(JSON.stringify(withoutPin));
  assert.equal(omitted?.name, "Demo");
  assert.equal(omitted?.sid, "demo-1");
  assert.equal(omitted?.pin, undefined);
  const empty = parsePlaygroundCard(JSON.stringify({ ...valid, pin: "" }));
  assert.equal(empty?.sid, "demo-1");
  assert.equal(empty?.pin, undefined);
  const blank = parsePlaygroundCard(JSON.stringify({ ...valid, pin: "   " }));
  assert.equal(blank?.sid, "demo-1");
  assert.equal(blank?.pin, undefined);
});

test("extracts a fenced playground card from a message", () => {
  const card = extractPlaygroundCard(
    `here\n\`\`\`playground\n${JSON.stringify(valid)}\n\`\`\``,
  );
  assert.deepEqual(card, valid);
  assert.equal(extractPlaygroundCard("```playground\nnot-json\n```"), null);
});

const HULA_PORT_HOLE = {
  hula: "playground",
  v: 1,
  name: "hula-port-hole",
  url: "https://hula-port-hole.hulapreview.com",
  sid: "oc-preview-hula-port-hole",
  stack: "main",
  expires: 1787564786,
};

test("parses the hula-port-hole custom-card payload (v:1, stack, expires)", () => {
  const raw = JSON.stringify(HULA_PORT_HOLE);
  assert.deepEqual(parsePlaygroundCard(raw), HULA_PORT_HOLE);
  assert.deepEqual(extractPlaygroundCard(raw), HULA_PORT_HOLE);
  assert.deepEqual(
    extractPlaygroundCard(`\`\`\`playground\n${raw}\n\`\`\``),
    HULA_PORT_HOLE,
  );
  assert.deepEqual(
    extractPlaygroundCard(`\`\`\`json\n${raw}\n\`\`\``),
    HULA_PORT_HOLE,
  );
});

