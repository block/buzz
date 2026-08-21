import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const projects = [
  ["desktop", "../scripts/check-e2e-bootstrap.mjs"],
  ["web", "../scripts/check-e2e-bootstrap.mjs"],
];
const imports = 'import { test, bootstrapE2ePage } from "../helpers/test";';
const spec = (body) =>
  `${imports}\ntest("x", async ({ page }) => { ${body} });`;

for (const [project, checkerPath] of projects) {
  const { checkSource, runCheck } = await import(checkerPath);
  const accepts = (source) => assert.equal(checkSource(source), undefined);
  const rejects = (source) => assert.notEqual(checkSource(source), undefined);

  test(`${project}: accepts direct, hook, and beforeNavigate bootstrap patterns`, () => {
    accepts(spec("await bootstrapE2ePage(page);"));
    accepts(
      `${imports}\ntest.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); });\ntest("x", async ({ page }) => { await page.title(); });`,
    );
    accepts(
      spec(
        "await bootstrapE2ePage(page, { beforeNavigate: async () => page.addInitScript(() => {}) });",
      ),
    );
  });

  test(`${project}: accepts module helpers and ratified containers`, () => {
    accepts(
      `${imports}\nasync function open(page) { return await bootstrapE2ePage(page); }\ntest("x", async ({ page }) => { const response = await open(page); void response; });`,
    );
    accepts(
      spec(
        "try { await bootstrapE2ePage(page); } finally { await page.title(); }",
      ),
    );
    accepts(
      spec(
        "try { await page.title(); } finally { await bootstrapE2ePage(page); }",
      ),
    );
    accepts(
      spec("for (const attempt of [0, 1]) { await bootstrapE2ePage(page); }"),
    );
    accepts(
      spec(
        "for (let attempt = 0; attempt < 2; attempt += 1) { await bootstrapE2ePage(page); }",
      ),
    );
  });

  test(`${project}: classifies indirect page fixture access and requires bootstrap`, () => {
    const cases = [
      [
        "whole fixture object",
        "fixtures",
        "await fixtures.page.title();",
        "const { page } = fixtures; await bootstrapE2ePage(page);",
      ],
      [
        "object rest",
        "{ ...rest }",
        "await rest.page.title();",
        "const { page } = rest; await bootstrapE2ePage(page);",
      ],
      [
        "body destructuring",
        "fixtures",
        "const { page } = fixtures; await page.title();",
        "const { page } = fixtures; await bootstrapE2ePage(page);",
      ],
    ];
    for (const [name, parameter, body, bootstrap] of cases) {
      const source = `${imports}\ntest("${name}", async (${parameter}) => { ${body} });`;
      assert.equal(
        checkSource(source),
        "must structurally await bootstrapE2ePage for every test, directly or through an applicable ancestor test.beforeEach",
      );
      accepts(
        `${imports}\ntest("${name}", async (${parameter}) => { ${bootstrap} ${body} });`,
      );
    }
  });

  test(`${project}: follows bounded fixture-object aliases and element access`, () => {
    const cases = [
      "const alias = fixtures; await alias.page.title();",
      'await fixtures["page"].title();',
      "const first = fixtures; const second = first; const { page } = second; await page.title();",
    ];
    for (const body of cases) {
      const source = `${imports}\ntest("x", async (fixtures) => { ${body} });`;
      assert.equal(
        checkSource(source),
        "must structurally await bootstrapE2ePage for every test, directly or through an applicable ancestor test.beforeEach",
      );
      accepts(
        `${imports}\ntest("x", async (fixtures) => { const { page } = fixtures; await bootstrapE2ePage(page); ${body} });`,
      );
    }
  });

  test(`${project}: requires bootstrap for each created page receiver`, () => {
    const unsafe = `${imports}
      test("x", async ({ page, context }) => {
        await bootstrapE2ePage(page);
        const second = await context.newPage();
        await second.goto("/");
      });`;
    assert.equal(
      checkSource(unsafe),
      "must structurally await bootstrapE2ePage for every test, directly or through an applicable ancestor test.beforeEach",
    );
    rejects(`${imports}
      test("x", async ({ page, context }) => {
        const second = await context.newPage();
        await bootstrapE2ePage(second, "/");
        await page.title();
      });`);
    accepts(`${imports}
      test("x", async ({ page, context }) => {
        const second = await context.newPage();
        await bootstrapE2ePage(page, "/");
        await bootstrapE2ePage(second, "/");
        await page.title();
      });`);
    rejects(`${imports}
      test("x", async ({ context }) => {
        const second = await context.newPage();
        await second.goto("/");
      });`);
    rejects(`${imports}
      test("x", async ({ context }) => {
        const second = await context.newPage();
        await second.goto("/");
        await bootstrapE2ePage(second, "/");
      });`);
    rejects(`${imports}
      test("x", async ({ page, context }) => {
        await bootstrapE2ePage(page);
        const second = await context.newPage();
        bootstrapE2ePage(second, "/");
        await second.goto("/");
      });`);
    accepts(`${imports}
      test("x", async ({ page, context }) => {
        await bootstrapE2ePage(page);
        const second = await context.newPage();
        await bootstrapE2ePage(second, "/");
      });`);
    rejects(`${imports}
      test("x", async ({ context }) => {
        const second = await context.newPage();
        await second.goto("/");
        await bootstrapE2ePage(second, "/");
        await second.goto("/later");
      });`);
    accepts(`${imports}
      test("x", async ({ context }) => {
        const second = await context.newPage();
        await bootstrapE2ePage(second, "/");
        await second.goto("/");
        await bootstrapE2ePage(second, "/later");
        await second.goto("/later");
      });`);

    rejects(`${imports}
      async function navigate(target) { await target.goto("/"); }
      test("fixture navigation before bootstrap", async ({ page }) => {
        await navigate(page);
        await bootstrapE2ePage(page, "/");
      });`);
    rejects(`${imports}
      async function readStorage(target) { await target.evaluate(() => localStorage.getItem("x")); }
      test("fixture storage before bootstrap", async ({ page }) => {
        await readStorage(page);
        await bootstrapE2ePage(page, "/");
      });`);
    accepts(`${imports}
      test("fixture setup before bootstrap", async ({ page }) => {
        await page.addInitScript(() => {});
        await bootstrapE2ePage(page, "/");
      });`);

    for (const body of [
      'const second = await context.newPage(); if (false) await bootstrapE2ePage(second); await second.goto("/");',
      'const second = await context.newPage(); async function never() { await bootstrapE2ePage(second); } await second.goto("/");',
      'let second; second = await context.newPage(); await second.goto("/");',
      'const second = await context.newPage(); second.goto("/");',
    ])
      rejects(
        `${imports}\n      test("x", async ({ context }) => { ${body} });`,
      );
  });

  test(`${project}: preserves helper event order and actual-argument mapping`, () => {
    rejects(`${imports}
      async function unsafeThenBootstrap(target) {
        await target.goto("/");
        await bootstrapE2ePage(target);
      }
      test("x", async ({ page }) => { await unsafeThenBootstrap(page); });`);
    accepts(`${imports}
      async function bootstrapThenUnsafe(target) {
        await bootstrapE2ePage(target);
        await target.goto("/");
      }
      test("x", async ({ page }) => { await bootstrapThenUnsafe(page); });`);
    rejects(`${imports}
      async function inner(safeTarget, unsafeTarget) {
        await bootstrapE2ePage(safeTarget);
        await unsafeTarget.goto("/");
      }
      async function outer(unsafeTarget, other) { await inner(other, unsafeTarget); }
      test("x", async ({ page }) => {
        const other = {};
        await outer(page, other);
      });`);
    accepts(`${imports}
      async function inner(target) { await bootstrapE2ePage(target); }
      async function outer(unused, target) { await inner(target); }
      test("x", async ({ page }) => { await outer(undefined, page); });`);
  });

  test(`${project}: conservatively checks executable conditional branches`, () => {
    rejects(
      spec(
        'if (condition) await page.goto("/"); await bootstrapE2ePage(page);',
      ),
    );
    rejects(
      spec(
        'switch (mode) { case "go": await page.goto("/"); break; default: break; } await bootstrapE2ePage(page);',
      ),
    );
    accepts(
      spec(
        'await bootstrapE2ePage(page); if (condition) await page.goto("/");',
      ),
    );
    accepts(
      spec(
        'await bootstrapE2ePage(page); switch (mode) { case "go": await page.goto("/"); break; default: break; }',
      ),
    );
    rejects(
      spec(
        'if (condition) await bootstrapE2ePage(page); await page.goto("/");',
      ),
    );
    accepts(
      spec('if (false) await page.goto("/"); await bootstrapE2ePage(page);'),
    );
  });

  test(`${project}: keeps hook coverage receiver-aware`, () => {
    rejects(`${imports}
      test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); });
      test("x", async ({ context }) => {
        const second = await context.newPage();
        await second.goto("/");
      });`);
    accepts(`${imports}
      test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); });
      test("x", async ({ page }) => { await page.title(); });`);
    accepts(`${imports}
      test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); });
      test("x", async ({ context }) => {
        const second = await context.newPage();
        await bootstrapE2ePage(second);
        await second.goto("/");
      });`);
  });

  test(`${project}: conservatively checks catch work`, () => {
    rejects(
      spec(
        'try { await page.title(); } catch { await page.goto("/"); } await bootstrapE2ePage(page);',
      ),
    );
    accepts(
      spec(
        'await bootstrapE2ePage(page); try { await page.title(); } catch { await page.goto("/"); }',
      ),
    );
    rejects(
      spec(
        'try { await page.title(); } catch { await bootstrapE2ePage(page); } await page.goto("/");',
      ),
    );
  });

  test(`${project}: propagates unawaited helper unsafe events`, () => {
    rejects(
      `${imports} async function read(target) { await target.evaluate(() => localStorage.getItem("x")); } test("x", async ({ page }) => { read(page); await bootstrapE2ePage(page); });`,
    );
    rejects(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { go(page); await bootstrapE2ePage(page); });`,
    );
    rejects(
      `${imports} async function boot(target) { await bootstrapE2ePage(target); } test("x", async ({ page }) => { boot(page); await page.goto("/"); });`,
    );
    accepts(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { await bootstrapE2ePage(page); await go(page); });`,
    );
    accepts(
      `${imports} async function setup(target) { await target.addInitScript(() => {}); } test("x", async ({ page }) => { setup(page); await bootstrapE2ePage(page); });`,
    );
  });

  test(`${project}: preserves safety events through parenthesized helper calls`, () => {
    for (const call of ["(go(page));", "await (go(page));"])
      rejects(
        `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { ${call} await bootstrapE2ePage(page); });`,
      );
    for (const call of ["(go(page));", "await (go(page));"])
      accepts(
        `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { await bootstrapE2ePage(page); ${call} });`,
      );
  });

  test(`${project}: preserves safety events through parenthesized helper callees`, () => {
    for (const call of ["(go)(page);", "await (go)(page);"])
      rejects(
        `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { ${call} await bootstrapE2ePage(page); });`,
      );
    rejects(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { await (go(page)); await bootstrapE2ePage(page); });`,
    );
    accepts(
      `${imports} async function setup(target) { await target.addInitScript(() => {}); } test("x", async ({ page }) => { await (setup)(page); await (bootstrapE2ePage)(page); });`,
    );
  });

  test(`${project}: preserves supported operations through member parentheses`, () => {
    for (const operation of [
      'await (page.goto)("/");',
      'await (page).goto("/");',
      'await ((page)["goto"])("/");',
      'await (page.evaluate)(() => localStorage.getItem("x"));',
    ])
      rejects(spec(`${operation} await bootstrapE2ePage(page);`));
    for (const operation of [
      'await (target.goto)("/");',
      'await (target).goto("/");',
      'await (target.evaluate)(() => localStorage.getItem("x"));',
    ])
      rejects(
        `${imports} async function unsafe(target) { ${operation} } test("x", async ({ page }) => { await unsafe(page); await bootstrapE2ePage(page); });`,
      );
    accepts(spec('await bootstrapE2ePage(page); await (page.goto)("/");'));
  });

  test(`${project}: recognizes parenthesized created-page operations`, () => {
    for (const creation of [
      "await (context.newPage)()",
      "await (context).newPage()",
      'await ((context)["newPage"])()',
      "await ((context.newPage)())",
    ])
      rejects(
        `${imports} test("x", async ({ context }) => { const second = ${creation}; await second.goto("/"); });`,
      );
    accepts(
      `${imports} test("x", async ({ context }) => { const second = await (context.newPage)(); await bootstrapE2ePage(second); await (second.goto)("/"); });`,
    );
  });

  test(`${project}: grants loop bootstrap credit only for guaranteed iterations`, () => {
    for (const loop of [
      "for (const value of values) await bootstrapE2ePage(page);",
      "for (const key in values) await bootstrapE2ePage(page);",
      "for (const value of [...[]]) await bootstrapE2ePage(page);",
      "for (const value of [...values]) await bootstrapE2ePage(page);",
      "for (const key in { ...{} }) await bootstrapE2ePage(page);",
      "for (const key in { ...values }) await bootstrapE2ePage(page);",
      "for (const key in { __proto__: null }) await bootstrapE2ePage(page);",
      'for (const key in { "__proto__": null }) await bootstrapE2ePage(page);',
      "for (const key in { [Symbol()]: 1 }) await bootstrapE2ePage(page);",
      "for (const key in { [Symbol.iterator]: 1 }) await bootstrapE2ePage(page);",
      "for (const key in { [key]: 1 }) await bootstrapE2ePage(page);",
      "for (const key in { [getKey()]: 1 }) await bootstrapE2ePage(page);",
      "for (const key in { [Symbol.iterator]() {} }) await bootstrapE2ePage(page);",
      "for (const key in { get [Symbol.iterator]() { return 1 } }) await bootstrapE2ePage(page);",
      "for (const key in { set [Symbol.iterator](value) {} }) await bootstrapE2ePage(page);",
      "for (const key in { [key]() {} }) await bootstrapE2ePage(page);",
      "for (const key in { get [key]() { return 1 } }) await bootstrapE2ePage(page);",
      "for (const key in { set [key](value) {} }) await bootstrapE2ePage(page);",
      "while (condition) await bootstrapE2ePage(page);",
      "for (; condition;) await bootstrapE2ePage(page);",
    ])
      rejects(spec(`${loop} await page.goto("/");`));
    rejects(
      spec(
        'let values = []; for (const value of values) await bootstrapE2ePage(page); values = [1]; await page.goto("/");',
      ),
    );
    accepts(
      spec(
        'const values = [1]; for (const value of values) await bootstrapE2ePage(page); await page.goto("/");',
      ),
    );
    rejects(
      spec(
        'const values = [...[]]; for (const value of values) await bootstrapE2ePage(page); await page.goto("/");',
      ),
    );
    for (const loop of [
      "for (const value of [1]) await bootstrapE2ePage(page);",
      "for (const value of [...[], 1]) await bootstrapE2ePage(page);",
      "for (const key in { ...{}, x: 1 }) await bootstrapE2ePage(page);",
      'for (const key in { ["__proto__"]: null }) await bootstrapE2ePage(page);',
      "for (const key in { [`x`]: 1 }) await bootstrapE2ePage(page);",
      "for (const key in { [1]: 1 }) await bootstrapE2ePage(page);",
      'for (const key in { ["x"]() {} }) await bootstrapE2ePage(page);',
      'for (const key in { get ["x"]() { return 1 } }) await bootstrapE2ePage(page);',
      'for (const key in { set ["x"](value) {} }) await bootstrapE2ePage(page);',
      "for (const key in { __proto__: null, x: 1 }) await bootstrapE2ePage(page);",
      'for (const value of "x") await bootstrapE2ePage(page);',
      "for (const key in { x: 1 }) await bootstrapE2ePage(page);",
      "for (;;) { await bootstrapE2ePage(page); break; }",
      "while (true) { await bootstrapE2ePage(page); break; }",
      "do { await bootstrapE2ePage(page); } while (condition);",
    ])
      accepts(spec(`${loop} await page.goto("/");`));
    rejects(
      spec(
        'for (const value of values) await page.goto("/"); await bootstrapE2ePage(page);',
      ),
    );
  });

  test(`${project}: invalidates unsafe events born on unmapped helper receivers`, () => {
    for (const operation of [
      'const alias = target; await alias.goto("/");',
      'const alias = target; await alias.evaluate(() => localStorage.getItem("x"));',
      'const unrelated = {}; await unrelated.goto("/");',
    ])
      rejects(
        `${imports} async function unsafe(target) { ${operation} } test("x", async ({ page }) => { await unsafe(page); await bootstrapE2ePage(page); });`,
      );
    rejects(
      `${imports} async function unsafe(target) { await target.goto("/"); } test("x", async ({ page }) => { await unsafe(page); await bootstrapE2ePage(page); });`,
    );
    accepts(
      `${imports} async function setup(target) { const alias = target; await alias.addInitScript(() => {}); } test("x", async ({ page }) => { await setup(page); await bootstrapE2ePage(page); });`,
    );
  });

  test(`${project}: fails closed when callback helper events cannot map to receivers`, () => {
    for (const actual of ["alias", "unrelated"])
      rejects(
        `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { const alias = page; const unrelated = {}; await go(${actual}); await bootstrapE2ePage(page); });`,
      );
    rejects(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { await go(page); await bootstrapE2ePage(page); });`,
    );
    accepts(
      `${imports} async function setup(target) { await target.addInitScript(() => {}); } test("x", async ({ page }) => { const alias = page; await setup(alias); await bootstrapE2ePage(page); });`,
    );
  });

  test(`${project}: classifies bounded element-access operations`, () => {
    rejects(spec('await page["goto"]("/"); await bootstrapE2ePage(page);'));
    rejects(
      spec(
        'await page["evaluate"](() => localStorage.getItem("x")); await bootstrapE2ePage(page);',
      ),
    );
    rejects(
      `${imports} test("x", async ({ context }) => { const second = await context["newPage"](); await second.goto("/"); });`,
    );
    accepts(spec('await bootstrapE2ePage(page); await page["goto"]("/");'));
    accepts(
      `${imports} test("x", async ({ context }) => { const second = await context["newPage"](); await bootstrapE2ePage(second); await second["goto"]("/"); });`,
    );
  });

  test(`${project}: fails closed for recursive and unmapped helpers`, () => {
    rejects(
      `${imports} async function setup(target) { await bootstrapE2ePage(target); await setup(target); } test("x", async ({ page }) => { await setup(page); });`,
    );
    rejects(
      `${imports} async function a(target) { await bootstrapE2ePage(target); await b(target); } async function b(target) { await a(target); } test("x", async ({ page }) => { await a(page); });`,
    );
    rejects(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { go(page as any); await bootstrapE2ePage(page); });`,
    );
    rejects(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { go((page)); await bootstrapE2ePage(page); });`,
    );
    rejects(
      `${imports} async function go(target) { await target.goto("/"); } test("x", async (fixtures) => { go(fixtures.page); const { page } = fixtures; await bootstrapE2ePage(page); });`,
    );
    accepts(
      `${imports} async function inner(target) { await bootstrapE2ePage(target); } async function outer(target) { await inner(target); } test("x", async ({ page }) => { await outer(page); });`,
    );
    accepts(
      `${imports} async function setup(target) { await target.addInitScript(() => {}); } test("x", async ({ page }) => { setup(page as any); await bootstrapE2ePage(page); });`,
    );
  });

  test(`${project}: rejects nested safety events mapped through local identifiers`, () => {
    rejects(
      `${imports} async function inner(target) { await target.goto("/"); } async function outer(target) { const alias = target; await inner(alias); } test("x", async ({ page }) => { await outer(page); await bootstrapE2ePage(page); });`,
    );
    rejects(
      `${imports} async function inner(target) { await target.goto("/"); } async function outer(target) { await inner(target); } test("x", async ({ page }) => { await outer(page); await bootstrapE2ePage(page); });`,
    );
    accepts(
      `${imports} async function inner(target) { await target.addInitScript(() => {}); } async function outer(target) { const alias = target; await inner(alias); } test("x", async ({ page }) => { await outer(page); await bootstrapE2ePage(page); });`,
    );
  });

  test(`${project}: applies created-page receiver enforcement in whole-file checks`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-created-page-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      for (const [name, body] of [
        [
          "wrong-fixture-receiver.spec.ts",
          'const second = await context.newPage(); await bootstrapE2ePage(second, "/"); await page.title();',
        ],
        [
          "safe-fixture-receiver.spec.ts",
          'const second = await context.newPage(); await bootstrapE2ePage(page, "/"); await bootstrapE2ePage(second, "/"); await page.title();',
        ],
        [
          "fixture-navigation-before-bootstrap.spec.ts",
          'await page.goto("/"); await bootstrapE2ePage(page, "/");',
        ],
        [
          "fixture-storage-before-bootstrap.spec.ts",
          'await page.evaluate(() => localStorage.getItem("x")); await bootstrapE2ePage(page, "/");',
        ],
        [
          "fixture-setup-before-bootstrap.spec.ts",
          'await page.addInitScript(() => {}); await bootstrapE2ePage(page, "/");',
        ],
        [
          "dead.spec.ts",
          'const second = await context.newPage(); if (false) await bootstrapE2ePage(second); await second.goto("/");',
        ],
        [
          "nested.spec.ts",
          'const second = await context.newPage(); async function never() { await bootstrapE2ePage(second); } await second.goto("/");',
        ],
        [
          "assigned.spec.ts",
          'let second; second = await context.newPage(); await second.goto("/");',
        ],
        [
          "unawaited-navigation.spec.ts",
          'const second = await context.newPage(); second.goto("/");',
        ],
        [
          "initial-navigation.spec.ts",
          'const second = await context.newPage(); await second.goto("/"); await bootstrapE2ePage(second, "/"); await second.goto("/later");',
        ],
        [
          "conditional-unsafe.spec.ts",
          'if (condition) await page.goto("/"); await bootstrapE2ePage(page);',
        ],
        [
          "conditional-safe.spec.ts",
          'await bootstrapE2ePage(page); if (condition) await page.goto("/");',
        ],
        [
          "switch-unsafe.spec.ts",
          'switch (mode) { case "go": await page.goto("/"); break; default: break; } await bootstrapE2ePage(page);',
        ],
        [
          "switch-safe.spec.ts",
          'await bootstrapE2ePage(page); switch (mode) { case "go": await page.goto("/"); break; default: break; }',
        ],
        [
          "initial-bootstrap.spec.ts",
          'await bootstrapE2ePage(page, "/"); const second = await context.newPage(); await bootstrapE2ePage(second, "/"); await second.goto("/"); await bootstrapE2ePage(second, "/later"); await second.goto("/later");',
        ],
      ])
        fs.writeFileSync(
          path.join(root, "tests/e2e", name),
          `${imports} test("x", async ({ page, context }) => { ${body} });`,
        );
      fs.writeFileSync(
        path.join(root, "tests/e2e/helper-order-unsafe.spec.ts"),
        `${imports} async function open(target) { await target.goto("/"); await bootstrapE2ePage(target); } test("x", async ({ page }) => { await open(page); });`,
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/helper-order-safe.spec.ts"),
        `${imports} async function open(target) { await bootstrapE2ePage(target); await target.goto("/"); } test("x", async ({ page }) => { await open(page); });`,
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/helper-mapping-unsafe.spec.ts"),
        `${imports} async function inner(safeTarget, unsafeTarget) { await bootstrapE2ePage(safeTarget); await unsafeTarget.goto("/"); } async function outer(unsafeTarget, other) { await inner(other, unsafeTarget); } test("x", async ({ page }) => { const other = {}; await outer(page, other); });`,
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/helper-mapping-safe.spec.ts"),
        `${imports} async function inner(target) { await bootstrapE2ePage(target); } async function outer(unused, target) { await inner(target); } test("x", async ({ page }) => { await outer(undefined, page); });`,
      );
      assert.equal(runCheck(root).length, 12);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: enforces helper-call boundaries in whole-file checks`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-helper-boundaries-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "parenthesized-awaited-unsafe.spec.ts",
          "await (go(page)); await bootstrapE2ePage(page);",
        ],
        [
          "parenthesized-unawaited-unsafe.spec.ts",
          "(go(page)); await bootstrapE2ePage(page);",
        ],
        [
          "parenthesized-awaited-safe.spec.ts",
          "await bootstrapE2ePage(page); await (go(page));",
        ],
        [
          "parenthesized-unawaited-safe.spec.ts",
          "await bootstrapE2ePage(page); (go(page));",
        ],
        [
          "callback-alias-unsafe.spec.ts",
          "const alias = page; await go(alias); await bootstrapE2ePage(page);",
        ],
        [
          "callback-unrelated-unsafe.spec.ts",
          "const unrelated = {}; await go(unrelated); await bootstrapE2ePage(page);",
        ],
        [
          "callback-direct-unsafe.spec.ts",
          "await go(page); await bootstrapE2ePage(page);",
        ],
        [
          "callback-setup-alias-safe.spec.ts",
          "const alias = page; await setup(alias); await bootstrapE2ePage(page);",
        ],
      ];
      for (const [name, body] of cases)
        fs.writeFileSync(
          path.join(root, "tests/e2e", name),
          `${imports} async function go(target) { await target.goto("/"); } async function setup(target) { await target.addInitScript(() => {}); } test("x", async ({ page }) => { ${body} });`,
        );
      assert.deepEqual(
        runCheck(root)
          .map((entry) => entry.split(":")[0])
          .sort(),
        [
          "callback-alias-unsafe.spec.ts",
          "callback-direct-unsafe.spec.ts",
          "callback-unrelated-unsafe.spec.ts",
          "parenthesized-awaited-unsafe.spec.ts",
          "parenthesized-unawaited-unsafe.spec.ts",
        ],
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: enforces event birth and callee parentheses in whole files`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-event-accounting-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "callee-awaited-unsafe.spec.ts",
          "await (go)(page); await bootstrapE2ePage(page);",
        ],
        [
          "callee-unawaited-unsafe.spec.ts",
          "(go)(page); await bootstrapE2ePage(page);",
        ],
        [
          "whole-call-control.spec.ts",
          "await (go(page)); await bootstrapE2ePage(page);",
        ],
        [
          "birth-alias-goto.spec.ts",
          "await aliasGoto(page); await bootstrapE2ePage(page);",
        ],
        [
          "birth-alias-storage.spec.ts",
          "await aliasStorage(page); await bootstrapE2ePage(page);",
        ],
        [
          "birth-unrelated-goto.spec.ts",
          "await unrelatedGoto(page); await bootstrapE2ePage(page);",
        ],
        [
          "birth-direct-control.spec.ts",
          "await go(page); await bootstrapE2ePage(page);",
        ],
        [
          "setup-control.spec.ts",
          "await (setup)(page); await (bootstrapE2ePage)(page);",
        ],
      ];
      const helpers = `
        async function go(target) { await target.goto("/"); }
        async function aliasGoto(target) { const alias = target; await alias.goto("/"); }
        async function aliasStorage(target) { const alias = target; await alias.evaluate(() => localStorage.getItem("x")); }
        async function unrelatedGoto(target) { const unrelated = {}; await unrelated.goto("/"); }
        async function setup(target) { const alias = target; await alias.addInitScript(() => {}); }
      `;
      for (const [name, body] of cases)
        fs.writeFileSync(
          path.join(root, "tests/e2e", name),
          `${imports} ${helpers} test("x", async ({ page }) => { ${body} });`,
        );
      assert.deepEqual(
        runCheck(root)
          .map((entry) => entry.split(":")[0])
          .sort(),
        [
          "birth-alias-goto.spec.ts",
          "birth-alias-storage.spec.ts",
          "birth-direct-control.spec.ts",
          "birth-unrelated-goto.spec.ts",
          "callee-awaited-unsafe.spec.ts",
          "callee-unawaited-unsafe.spec.ts",
          "whole-call-control.spec.ts",
        ],
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: enforces member parentheses and loop accounting in whole files`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-conservative-accounting-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "member-callee.spec.ts",
          'await (page.goto)("/"); await bootstrapE2ePage(page);',
        ],
        [
          "member-receiver.spec.ts",
          'await (page).goto("/"); await bootstrapE2ePage(page);',
        ],
        [
          "member-storage.spec.ts",
          'await (page.evaluate)(() => localStorage.getItem("x")); await bootstrapE2ePage(page);',
        ],
        [
          "helper-member.spec.ts",
          "await unsafe(page); await bootstrapE2ePage(page);",
        ],
        [
          "created-member.spec.ts",
          'const second = await (context.newPage)(); await second.goto("/");',
        ],
        [
          "created-safe.spec.ts",
          'await bootstrapE2ePage(page); const second = await (context).newPage(); await bootstrapE2ePage(second); await (second.goto)("/");',
        ],
        [
          "dynamic-for-of.spec.ts",
          'for (const value of values) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "dynamic-while.spec.ts",
          'while (condition) { await bootstrapE2ePage(page); break; } await page.goto("/");',
        ],
        [
          "spread-array.spec.ts",
          'for (const value of [...[]]) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "spread-object.spec.ts",
          'for (const key in { ...{} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "temporal-loop.spec.ts",
          'let values = []; for (const value of values) await bootstrapE2ePage(page); values = [1]; await page.goto("/");',
        ],
        [
          "proto-identifier.spec.ts",
          'for (const key in { __proto__: null }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "proto-string.spec.ts",
          'for (const key in { "__proto__": null }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "proto-computed-safe.spec.ts",
          'for (const key in { ["__proto__"]: null }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-number-safe.spec.ts",
          'for (const key in { [1]: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-symbol.spec.ts",
          'for (const key in { [Symbol()]: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-symbol-member.spec.ts",
          'for (const key in { [Symbol.iterator]: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-bound.spec.ts",
          'const key = Symbol(); for (const item in { [key]: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-call.spec.ts",
          'for (const key in { [getKey()]: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-symbol-method.spec.ts",
          'for (const key in { [Symbol.iterator]() {} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-symbol-getter.spec.ts",
          'for (const key in { get [Symbol.iterator]() { return 1 } }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-symbol-setter.spec.ts",
          'for (const key in { set [Symbol.iterator](value) {} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-dynamic-method.spec.ts",
          'for (const key in { [dynamicKey]() {} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-dynamic-getter.spec.ts",
          'for (const key in { get [dynamicKey]() { return 1 } }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-dynamic-setter.spec.ts",
          'for (const key in { set [dynamicKey](value) {} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-static-method-safe.spec.ts",
          'for (const key in { ["x"]() {} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-static-getter-safe.spec.ts",
          'for (const key in { get ["x"]() { return 1 } }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "computed-static-setter-safe.spec.ts",
          'for (const key in { set ["x"](value) {} }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "proto-mixed-safe.spec.ts",
          'for (const key in { __proto__: null, x: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "literal-loop-safe.spec.ts",
          'for (const value of [1]) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "concrete-spread-array-safe.spec.ts",
          'for (const value of [...[], 1]) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "concrete-spread-object-safe.spec.ts",
          'for (const key in { ...{}, x: 1 }) await bootstrapE2ePage(page); await page.goto("/");',
        ],
        [
          "do-loop-safe.spec.ts",
          'do { await bootstrapE2ePage(page); } while (condition); await page.goto("/");',
        ],
      ];
      for (const [name, body] of cases)
        fs.writeFileSync(
          path.join(root, "tests/e2e", name),
          `${imports} async function unsafe(target) { await (target.evaluate)(() => localStorage.getItem("x")); } test("x", async ({ page, context }) => { ${body} });`,
        );
      assert.deepEqual(
        runCheck(root)
          .map((entry) => entry.split(":")[0])
          .sort(),
        [
          "computed-bound.spec.ts",
          "computed-call.spec.ts",
          "computed-dynamic-getter.spec.ts",
          "computed-dynamic-method.spec.ts",
          "computed-dynamic-setter.spec.ts",
          "computed-symbol-getter.spec.ts",
          "computed-symbol-member.spec.ts",
          "computed-symbol-method.spec.ts",
          "computed-symbol-setter.spec.ts",
          "computed-symbol.spec.ts",
          "created-member.spec.ts",
          "dynamic-for-of.spec.ts",
          "dynamic-while.spec.ts",
          "helper-member.spec.ts",
          "member-callee.spec.ts",
          "member-receiver.spec.ts",
          "member-storage.spec.ts",
          "proto-identifier.spec.ts",
          "proto-string.spec.ts",
          "spread-array.spec.ts",
          "spread-object.spec.ts",
          "temporal-loop.spec.ts",
        ],
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: applies receiver-aware hooks and catch checks in whole files`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-hook-catch-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "hook-created-unsafe.spec.ts",
          'test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); }); test("x", async ({ context }) => { const second = await context.newPage(); await second.goto("/"); });',
        ],
        [
          "hook-fixture-safe.spec.ts",
          'test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); }); test("x", async ({ page }) => { await page.title(); });',
        ],
        [
          "hook-created-safe.spec.ts",
          'test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); }); test("x", async ({ context }) => { const second = await context.newPage(); await bootstrapE2ePage(second); await second.goto("/"); });',
        ],
        [
          "catch-unsafe.spec.ts",
          'test("x", async ({ page }) => { try { await page.title(); } catch { await page.goto("/"); } await bootstrapE2ePage(page); });',
        ],
        [
          "catch-safe.spec.ts",
          'test("x", async ({ page }) => { await bootstrapE2ePage(page); try { await page.title(); } catch { await page.goto("/"); } });',
        ],
        [
          "catch-bootstrap.spec.ts",
          'test("x", async ({ page }) => { try { await page.title(); } catch { await bootstrapE2ePage(page); } await page.goto("/"); });',
        ],
      ];
      for (const [name, body] of cases)
        fs.writeFileSync(
          path.join(root, "tests/e2e", name),
          `${imports} ${body}`,
        );
      assert.deepEqual(
        runCheck(root)
          .map((entry) => entry.split(":")[0])
          .sort(),
        [
          "catch-bootstrap.spec.ts",
          "catch-unsafe.spec.ts",
          "hook-created-unsafe.spec.ts",
        ],
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: applies unawaited and element-access checks in whole files`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-unawaited-element-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "unawaited-storage.spec.ts",
          'async function read(target) { await target.evaluate(() => localStorage.getItem("x")); } test("x", async ({ page }) => { read(page); await bootstrapE2ePage(page); });',
        ],
        [
          "unawaited-goto.spec.ts",
          'async function go(target) { await target.goto("/"); } test("x", async ({ page }) => { go(page); await bootstrapE2ePage(page); });',
        ],
        [
          "unawaited-safe.spec.ts",
          'async function setup(target) { await target.addInitScript(() => {}); } test("x", async ({ page }) => { setup(page); await bootstrapE2ePage(page); });',
        ],
        [
          "element-goto.spec.ts",
          'test("x", async ({ page }) => { await page["goto"]("/"); await bootstrapE2ePage(page); });',
        ],
        [
          "element-created.spec.ts",
          'test("x", async ({ context }) => { const second = await context["newPage"](); await second.goto("/"); });',
        ],
        [
          "element-safe.spec.ts",
          'test("x", async ({ page }) => { await bootstrapE2ePage(page); await page["goto"]("/"); });',
        ],
      ];
      for (const [name, body] of cases)
        fs.writeFileSync(
          path.join(root, "tests/e2e", name),
          `${imports} ${body}`,
        );
      assert.deepEqual(
        runCheck(root)
          .map((x) => x.split(":")[0])
          .sort(),
        [
          "element-created.spec.ts",
          "element-goto.spec.ts",
          "unawaited-goto.spec.ts",
          "unawaited-storage.spec.ts",
        ],
      );
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: applies recursion and unmapped guards in whole files`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-recursion-unmapped-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "direct.spec.ts",
          'async function setup(target){await bootstrapE2ePage(target);await setup(target);} test("x",async({page})=>{await setup(page);});',
        ],
        [
          "mutual.spec.ts",
          'async function a(target){await bootstrapE2ePage(target);await b(target);} async function b(target){await a(target);} test("x",async({page})=>{await a(page);});',
        ],
        [
          "as.spec.ts",
          'async function go(target){await target.goto("/");} test("x",async({page})=>{go(page as any);await bootstrapE2ePage(page);});',
        ],
        [
          "paren.spec.ts",
          'async function go(target){await target.goto("/");} test("x",async({page})=>{go((page));await bootstrapE2ePage(page);});',
        ],
        [
          "property.spec.ts",
          'async function go(target){await target.goto("/");} test("x",async(fixtures)=>{go(fixtures.page);const {page}=fixtures;await bootstrapE2ePage(page);});',
        ],
        [
          "safe.spec.ts",
          'async function inner(target){await bootstrapE2ePage(target);} test("x",async({page})=>{await inner(page);});',
        ],
      ];
      for (const [n, b] of cases)
        fs.writeFileSync(path.join(root, "tests/e2e", n), `${imports} ${b}`);
      assert.equal(runCheck(root).length, 5);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: applies local-identifier mapping guards in whole files`, () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "e2e-bootstrap-local-map-"),
    );
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      const cases = [
        [
          "alias.spec.ts",
          'async function inner(target){await target.goto("/");} async function outer(target){const alias=target;await inner(alias);} test("x",async({page})=>{await outer(page);await bootstrapE2ePage(page);});',
        ],
        [
          "direct.spec.ts",
          'async function inner(target){await target.goto("/");} async function outer(target){await inner(target);} test("x",async({page})=>{await outer(page);await bootstrapE2ePage(page);});',
        ],
        [
          "setup.spec.ts",
          'async function inner(target){await target.addInitScript(()=>{});} async function outer(target){const alias=target;await inner(alias);} test("x",async({page})=>{await outer(page);await bootstrapE2ePage(page);});',
        ],
      ];
      for (const [n, b] of cases)
        fs.writeFileSync(path.join(root, "tests/e2e", n), `${imports} ${b}`);
      assert.equal(runCheck(root).length, 2);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: ignores destructuring property keys with different bound locals`, () => {
    accepts(
      spec(
        "const { test: localTest, bootstrapE2ePage: localBootstrap } = helpers; void localTest; void localBootstrap; await bootstrapE2ePage(page);",
      ),
    );
  });

  for (const [name, source] of [
    [
      ".ts helper suffix",
      'import { test, bootstrapE2ePage } from "../helpers/test.ts"; test("x", async () => {});',
    ],
    [
      "import alias",
      'import { test as e2eTest, bootstrapE2ePage } from "../helpers/test"; e2eTest("x", async () => { await bootstrapE2ePage(); });',
    ],
    [
      "re-export",
      'import { test, bootstrapE2ePage } from "./reexport"; test("x", async () => { await bootstrapE2ePage(); });',
    ],
    [
      "dynamic import",
      'const { test, bootstrapE2ePage } = await import("../helpers/test"); test("x", async () => { await bootstrapE2ePage(); });',
    ],
    [
      "dead-code call",
      `${imports}\nasync function never(page) { await bootstrapE2ePage(page); } test("x", async () => {});`,
    ],
    [
      "comment",
      `${imports}\n// await bootstrapE2ePage(page)\ntest("x", async () => {});`,
    ],
    [
      "string",
      `${imports}\nconst hint = "await bootstrapE2ePage(page)"; test("x", async () => {});`,
    ],
    [
      "mixed safe and unsafe tests",
      `${imports}\ntest("safe", async ({ page }) => { await bootstrapE2ePage(page); }); test("unsafe", async ({ page }) => { await page.title(); });`,
    ],
    ["missing bootstrap receiver", spec("await bootstrapE2ePage();")],
    ["non-identifier bootstrap receiver", spec('await bootstrapE2ePage("/");')],
    ["if-wrapped call", spec("if (true) { await bootstrapE2ePage(page); }")],
    [
      "catch-only call",
      spec(
        "try { await page.title(); } catch { await bootstrapE2ePage(page); }",
      ),
    ],
    [
      "zero-iteration false loop",
      spec("for (; false;) { await bootstrapE2ePage(page); }"),
    ],
    [
      "zero-iteration empty iterable",
      spec("for (const value of []) { await bootstrapE2ePage(page); }"),
    ],
    [
      "uncalled nested function",
      spec("async function setup() { await bootstrapE2ePage(page); }"),
    ],
    [
      "unawaited helper",
      `${imports}\nasync function open(page) { await bootstrapE2ePage(page); }\ntest("x", async ({ page }) => { open(page); });`,
    ],
    [
      "fake test.extend",
      `${imports}\ntest.extend("x", async ({ page }) => { await bootstrapE2ePage(page); });`,
    ],
    [
      "misplaced describe hook",
      `${imports}\ntest.describe("inside", () => { test.beforeEach(async ({ page }) => { await bootstrapE2ePage(page); }); test("covered", async () => {}); }); test("outside", async () => {});`,
    ],
    [
      "shadowed bootstrap",
      `${imports}\ntest("x", async ({ page }) => { const bootstrapE2ePage = async () => {}; await bootstrapE2ePage(page); });`,
    ],
    [
      "object shorthand binding shadow",
      spec(
        "const { bootstrapE2ePage } = helpers; await bootstrapE2ePage(page);",
      ),
    ],
    [
      "nested object alias binding shadow",
      spec(
        "const { bootstrap: { run: bootstrapE2ePage } } = helpers; await bootstrapE2ePage(page);",
      ),
    ],
    [
      "nested array default binding shadow",
      spec("const [[test = fallback]] = values; await bootstrapE2ePage(page);"),
    ],
    [
      "object rest binding shadow",
      spec(
        "const { ignored, ...bootstrapE2ePage } = helpers; await bootstrapE2ePage(page);",
      ),
    ],
    [
      "destructured function parameter shadow",
      spec(
        "function configure({ nested: [bootstrapE2ePage] }) {} await bootstrapE2ePage(page);",
      ),
    ],
    [
      "destructured arrow parameter shadow",
      spec(
        "const configure = ([{ test }]) => {}; await bootstrapE2ePage(page);",
      ),
    ],
    [
      "destructured method parameter shadow",
      spec(
        "class Harness { configure({ helper: { bootstrapE2ePage = fallback } }) {} } await bootstrapE2ePage(page);",
      ),
    ],
    [
      "parenthesized false condition",
      spec("while (((false))) { await bootstrapE2ePage(page); }"),
    ],
    [
      "parenthesized empty array iterable",
      spec("for (const value of (([]))) { await bootstrapE2ePage(page); }"),
    ],
    [
      "parenthesized empty string iterable",
      spec('for (const value of ((""))) { await bootstrapE2ePage(page); }'),
    ],
    [
      "parenthesized empty object for-in",
      spec("for (const key in (({}))) { await bootstrapE2ePage(page); }"),
    ],
  ]) {
    test(`${project}: rejects ${name}`, () => rejects(source));
  }

  test(`${project}: classifies non-browser harnesses structurally`, () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "e2e-bootstrap-"));
    try {
      fs.mkdirSync(path.join(root, "tests/e2e"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/agents-everywhere.live.spec.ts"),
        'import { test } from "../helpers/test"; test("relay", async () => {});',
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/renamed-cli-harness.spec.ts"),
        'import { test } from "../helpers/test"; test("relay", async () => {});',
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/agents-everywhere.live.perf.ts"),
        'import { test } from "../helpers/test"; test("browser", async ({ page }) => { await page.title(); });',
      );
      assert.deepEqual(runCheck(root), [
        "agents-everywhere.live.perf.ts: must directly import unaliased test and bootstrapE2ePage from the canonical tests/helpers/test module",
      ]);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  test(`${project}: discovers nested specs with a canonical relative import`, () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "e2e-bootstrap-"));
    try {
      fs.mkdirSync(path.join(root, "tests/e2e/nested"), { recursive: true });
      fs.mkdirSync(path.join(root, "tests/helpers"), { recursive: true });
      fs.writeFileSync(
        path.join(root, "tests/helpers/test.ts"),
        "export {};\n",
      );
      fs.writeFileSync(
        path.join(root, "tests/e2e/nested/example.spec.ts"),
        'import { test, bootstrapE2ePage } from "../../helpers/test"; test("x", async ({ page }) => { await bootstrapE2ePage(page); });',
      );
      assert.deepEqual(runCheck(root), []);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
}

for (const [project, checkerPath] of projects) {
  const { checkSource } = await import(checkerPath);
  test(`${project}: traverses canonical parameterized registration containers only`, () => {
    assert.equal(
      checkSource(`${imports}
        for (const name of ["one", "two"]) test(name, async ({ page }) => { await bootstrapE2ePage(page); });
        ["three"].forEach((name) => test(name, async ({ page }) => { await bootstrapE2ePage(page); }));
        ["four"].map((name) => test(name, async ({ page }) => { await bootstrapE2ePage(page); }));`),
      undefined,
    );
    assert.notEqual(
      checkSource(`${imports}
        function registerLater() { test("hidden", async ({ page }) => { await bootstrapE2ePage(page); }); }
        test("unsafe", async ({ page }) => { await page.title(); });`),
      undefined,
    );
  });
}
