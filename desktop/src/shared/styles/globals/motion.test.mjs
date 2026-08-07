import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const motionCss = readFileSync(
  new URL("./motion.css", import.meta.url),
  "utf8",
);
const tailwindConfig = readFileSync(
  new URL("../../../../tailwind.config.js", import.meta.url),
  "utf8",
);

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return [".css", ".ts", ".tsx"].includes(extname(path)) ? [path] : [];
  });
}

test("conversation arrival uses shared motion tokens", () => {
  assert.match(motionCss, /--motion-duration-arrival:\s*500ms/);
  assert.match(motionCss, /--motion-ease-arrival:/);
  assert.match(
    motionCss,
    /\.motion-enter-conversation\s*\{[\s\S]*var\(--motion-duration-arrival\)[\s\S]*var\(--motion-ease-arrival\)/,
  );
});

test("conversation arrival has a reduced-motion treatment", () => {
  assert.match(
    motionCss,
    /@media \(prefers-reduced-motion: reduce\)[\s\S]*\.motion-enter-conversation/,
  );
});

test("shared state-transition utilities resolve through motion tokens", () => {
  assert.match(
    tailwindConfig,
    /"motion-feedback":\s*"var\(--motion-duration-instant\)"/,
  );
  assert.match(
    tailwindConfig,
    /"motion-state":\s*"var\(--motion-duration-fast\)"/,
  );
  assert.match(
    tailwindConfig,
    /"motion-standard":\s*"var\(--motion-ease-standard\)"/,
  );
});

test("semantic motion durations collapse under reduced motion", () => {
  const reducedMotionBlock = motionCss.match(
    /@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*)\}\s*\/\*/,
  )?.[1];

  assert.ok(reducedMotionBlock, "expected a reduced-motion media block");
  for (const token of ["instant", "fast", "standard", "arrival"]) {
    assert.match(
      reducedMotionBlock,
      new RegExp(`--motion-duration-${token}:\\s*1ms`),
    );
  }
});

test("interactive styles never transition every property", () => {
  const sourceRoot = fileURLToPath(new URL("../../../", import.meta.url));
  const violations = sourceFiles(sourceRoot)
    .flatMap((path) => {
      const source = readFileSync(path, "utf8");
      return (
        source
          .match(/transition-all|transition(?:-property)?\s*:\s*all/g)
          ?.map(() => path) ?? []
      );
    })
    .sort();

  assert.deepEqual(violations, []);
});
