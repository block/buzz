#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

function requiredPlaceholder(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`missing required OpenShell provider value: ${name}`);
  }
  if (!value.startsWith("openshell:resolve:env:")) {
    throw new Error(`${name} is not an opaque OpenShell provider placeholder`);
  }
  return value;
}

function base64url(value) {
  return Buffer.from(JSON.stringify(value)).toString("base64url");
}

const codexHome = process.env.CODEX_HOME ?? path.join(process.env.HOME ?? "", ".codex");
if (!path.isAbsolute(codexHome)) {
  throw new Error("CODEX_HOME (or HOME) must resolve to an absolute path");
}

const accessToken = requiredPlaceholder("CODEX_AUTH_ACCESS_TOKEN");
const accountId = requiredPlaceholder("CODEX_AUTH_ACCOUNT_ID");
const now = Math.floor(Date.now() / 1000);
const fallbackIdToken = [
  base64url({ alg: "none", typ: "JWT" }),
  base64url({
    iss: "https://auth.openai.com",
    aud: "codex",
    sub: "openshell-agent",
    email: "agent@openshell.local",
    iat: now,
    exp: now + 3600,
  }),
  "placeholder",
].join(".");

const auth = {
  auth_mode: "chatgpt",
  OPENAI_API_KEY: null,
  tokens: {
    id_token: fallbackIdToken,
    access_token: accessToken,
    refresh_token: "gateway-managed-refresh-token",
    account_id: accountId,
  },
  last_refresh: new Date().toISOString(),
};

fs.mkdirSync(codexHome, { recursive: true, mode: 0o700 });
fs.chmodSync(codexHome, 0o700);
const authPath = path.join(codexHome, "auth.json");
if (fs.existsSync(authPath) && fs.lstatSync(authPath).isSymbolicLink()) {
  throw new Error(`refusing to replace symlink: ${authPath}`);
}
const temporaryPath = path.join(codexHome, `.auth.json.${process.pid}.tmp`);
try {
  fs.writeFileSync(temporaryPath, `${JSON.stringify(auth, null, 2)}\n`, {
    encoding: "utf8",
    flag: "wx",
    mode: 0o600,
  });
  fs.renameSync(temporaryPath, authPath);
  fs.chmodSync(authPath, 0o600);
} finally {
  fs.rmSync(temporaryPath, { force: true });
}

process.stderr.write(`materialized gateway-backed Codex auth at ${authPath}\n`);
