import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

test("PinnedSiteScreen keeps deep startUrl after pending clear (no home clobber)", () => {
  const dir = dirname(fileURLToPath(import.meta.url));
  const source = readFileSync(join(dir, "PinnedSiteScreen.tsx"), "utf8");
  assert.match(
    source,
    /Pending\/navOpenUrl always win over sticky navClearsDeepLink/,
  );
  assert.match(
    source,
    /setStartUrl\(\(prev\) => prev \|\| pin\.url\)/,
  );
  assert.match(
    source,
    /goPinnedSite|navOpenUrl|pinnedSiteOpenUrl/,
  );
  assert.match(
    source,
    /clearPinnedSiteOpenUrl\(id, appliedUrl\)/,
  );
  assert.match(
    source,
    /const pending =[\s\S]*navClearsDeepLink/,
    "pending/deep URL is resolved before sticky home-clear branch",
  );
});
