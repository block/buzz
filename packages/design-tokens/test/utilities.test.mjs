import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { compile } from "tailwindcss";
import { ROLE_GROUPS } from "../src/registry.ts";
const source = readFileSync(
  new URL("../src/tokens.css", import.meta.url),
  "utf8",
);
test("generated utilities bind the same roles as the registry and contrast audit", async () => {
  const roles = ROLE_GROUPS.flatMap((g) => g.roles).filter(
    (r) => !["bg-app"].includes(r.token),
  );
  const compiler = await compile(source + "\n@tailwind utilities;");
  const css = compiler.build(roles.map((r) => r.token));
  for (const role of roles) {
    const start = css.indexOf("." + role.token + " {");
    assert.notEqual(start, -1, "missing utility " + role.token);
    const rule = css.slice(start, css.indexOf("}", start));
    assert.ok(
      rule.includes("var(" + role.variable + ")"),
      role.token + " must bind " + role.variable + "; got " + rule,
    );
  }
});
test("off-palette utilities cannot silently bring another palette into the system", async () => {
  const compiler = await compile(source + "\n@tailwind utilities;");
  const css = compiler.build(["text-gray-500", "bg-red-500"]);
  assert.ok(!css.includes(".text-gray-500"));
  assert.ok(!css.includes(".bg-red-500"));
});
