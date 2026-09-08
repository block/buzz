import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadEnv } from "vite";

const desktopRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const vitePackageJsonPath = fileURLToPath(
  import.meta.resolve("vite/package.json"),
);
const vitePackage = JSON.parse(readFileSync(vitePackageJsonPath, "utf8"));
const viteEntrypoint = path.resolve(
  path.dirname(vitePackageJsonPath),
  vitePackage.bin.vite,
);

function buildVariant({ internal, output }) {
  const env = {
    ...process.env,
    // Pin both children explicitly. Deleting the OSS value lets Vite reload
    // `=1` from .env.local or a mode-specific env file.
    VITE_BUZZ_BESTIE: internal ? "1" : "0",
  };

  const result = spawnSync(
    process.execPath,
    [viteEntrypoint, "build", "--outDir", output, "--emptyOutDir"],
    {
      cwd: desktopRoot,
      env,
      stdio: "inherit",
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${internal ? "internal" : "OSS"} desktop build failed with status ${result.status}`,
    );
  }
}

function emittedText(root) {
  const chunks = [];
  const visit = (candidate) => {
    const stat = statSync(candidate);
    if (stat.isDirectory()) {
      for (const child of readdirSync(candidate)) {
        visit(path.join(candidate, child));
      }
      return;
    }
    if (/\.(?:css|html|js|json)$/u.test(candidate)) {
      chunks.push(readFileSync(candidate, "utf8"));
    }
  };
  visit(root);
  return chunks.join("\n");
}

export function assertArtifactContract({ ossOutput, internalOutput }) {
  const ossText = emittedText(ossOutput);
  const internalText = emittedText(internalOutput);
  const protectedContent = /\bbestie\b|chief of staff|builtin:bestie/iu;
  const internalManifestMarker =
    "Try a personal agent that is always close at hand";

  if (protectedContent.test(ossText)) {
    throw new Error(
      "Official OSS desktop artifact contains protected Bestie/Chief content",
    );
  }
  if (!internalText.includes(internalManifestMarker)) {
    throw new Error(
      "Protected internal desktop artifact is missing the Bestie manifest",
    );
  }
}

/** Resolve the requested output with the same precedence used by Vite config. */
export function selectInternalVariant({ processEnv, modeEnv }) {
  return (processEnv.VITE_BUZZ_BESTIE ?? modeEnv.VITE_BUZZ_BESTIE) === "1";
}

/** Build and inspect both graphs, leaving the requested variant in dist. */
export function buildArtifactMatrix({
  selectedInternalVariant,
  selectedOutput,
  alternateOutput,
  build = buildVariant,
}) {
  // Build the unselected variant outside dist first, then leave the requested
  // variant in dist for Vite/Tauri's ordinary packaging contract.
  build({
    internal: !selectedInternalVariant,
    output: alternateOutput,
  });
  build({
    internal: selectedInternalVariant,
    output: selectedOutput,
  });

  assertArtifactContract({
    ossOutput: selectedInternalVariant ? alternateOutput : selectedOutput,
    internalOutput: selectedInternalVariant ? selectedOutput : alternateOutput,
  });
}

function main() {
  const selectedInternalVariant = selectInternalVariant({
    processEnv: process.env,
    modeEnv: loadEnv("production", desktopRoot, ""),
  });
  const scratchRoot = mkdtempSync(
    path.join(tmpdir(), "buzz-protected-feature-artifacts-"),
  );
  const selectedOutput = process.env.BUZZ_PROTECTED_BUILD_OUTPUT
    ? path.resolve(process.env.BUZZ_PROTECTED_BUILD_OUTPUT)
    : path.join(desktopRoot, "dist");
  const alternateOutput = path.join(scratchRoot, "alternate");

  try {
    buildArtifactMatrix({
      selectedInternalVariant,
      selectedOutput,
      alternateOutput,
    });
  } finally {
    rmSync(scratchRoot, { recursive: true, force: true });
  }

  console.log(
    `Protected feature artifact matrix passed; dist contains the ${selectedInternalVariant ? "internal" : "OSS"} variant.`,
  );
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main();
}
