import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const SEMVER_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

export function createCodexLabLatestManifest({
  version,
  signature,
  url,
  allowInsecure = false,
  pubDate = new Date().toISOString(),
}) {
  if (!SEMVER_RE.test(version ?? "")) {
    throw new Error(`version must be semantic; got ${JSON.stringify(version)}`);
  }
  if (!(signature ?? "").trim()) {
    throw new Error("signature must not be empty");
  }

  const artifactUrl = new URL(url);
  if (artifactUrl.protocol !== "https:" && !(allowInsecure && artifactUrl.protocol === "http:")) {
    throw new Error(`updater artifact URL must use HTTPS; got ${url}`);
  }
  if (Number.isNaN(Date.parse(pubDate))) {
    throw new Error(`pubDate must be an ISO timestamp; got ${pubDate}`);
  }

  return {
    version,
    notes: `Buzz Codex Lab v${version}`,
    pub_date: pubDate,
    platforms: {
      "windows-x86_64": {
        signature: signature.trim(),
        url: artifactUrl.toString(),
      },
    },
  };
}

function requireOption(values, name) {
  const value = values[name];
  if (!value) throw new Error(`--${name} is required`);
  return value;
}

function main() {
  const { values } = parseArgs({
    options: {
      version: { type: "string" },
      "signature-file": { type: "string" },
      url: { type: "string" },
      output: { type: "string" },
      "pub-date": { type: "string" },
      "allow-insecure": { type: "boolean", default: false },
    },
  });

  const signaturePath = requireOption(values, "signature-file");
  const manifest = createCodexLabLatestManifest({
    version: requireOption(values, "version"),
    signature: readFileSync(signaturePath, "utf8"),
    url: requireOption(values, "url"),
    allowInsecure: values["allow-insecure"],
    pubDate: values["pub-date"],
  });
  const json = `${JSON.stringify(manifest, null, 2)}\n`;
  const output = values.output;
  if (output) {
    writeFileSync(output, json);
  } else {
    process.stdout.write(json);
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main();
}
