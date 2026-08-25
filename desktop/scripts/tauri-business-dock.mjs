import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { loadEnv } from "vite";

const require = createRequire(import.meta.url);

export function normalizeBusinessOrigin(value) {
  if (!value?.trim()) return null;
  const url = new URL(value.trim());
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  ) {
    throw new Error(
      "VITE_BUSINESS_APP_ORIGIN must be an HTTP(S) origin without credentials, path, query, or fragment",
    );
  }
  return url.origin;
}

export function buildBusinessDockCsp(baseCsp, configuredOrigin) {
  const origin = normalizeBusinessOrigin(configuredOrigin);
  const directives = baseCsp
    .split(";")
    .map((directive) => directive.trim())
    .filter(Boolean)
    .filter((directive) => !directive.startsWith("frame-src "));
  directives.push(`frame-src 'self'${origin ? ` ${origin}` : ""}`);
  return directives.join("; ");
}

function configArguments(args) {
  const values = [];
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index];
    if (value === "--config" && args[index + 1]) values.push(args[index + 1]);
    else if (value.startsWith("--config=")) values.push(value.slice(9));
  }
  return values;
}

export function desktopBuildChannel(args) {
  if (args[0] === "dev") return "development";
  return configArguments(args).some((value) =>
    value.includes("tauri.dev.conf.json"),
  )
    ? "development"
    : "production";
}

function isTestHostname(hostname) {
  return (
    hostname === "localhost" ||
    hostname === "127.0.0.1" ||
    hostname.endsWith(".localhost") ||
    hostname.endsWith(".test")
  );
}

function requiredOidcValues(env) {
  return [
    env.VITE_OIDC_ISSUER,
    env.VITE_OIDC_CLIENT_ID,
    env.VITE_OIDC_REDIRECT_URI,
    env.VITE_OIDC_POST_LOGOUT_REDIRECT_URI,
  ].map((value) => value?.trim() ?? "");
}

export function validateDesktopBuildEnvironment(channel, env) {
  const oidcValues = requiredOidcValues(env);
  const oidcConfigured = oidcValues.some(Boolean);
  if (oidcConfigured && oidcValues.some((value) => !value)) {
    throw new Error("Desktop OIDC configuration is incomplete.");
  }

  const expectedScheme = channel === "development" ? "buzz-dev:" : "buzz:";
  if (oidcConfigured) {
    const issuer = new URL(oidcValues[0]);
    const redirect = new URL(oidcValues[2]);
    const logout = new URL(oidcValues[3]);
    if (redirect.href !== `${expectedScheme}//auth/callback`) {
      throw new Error(
        `${channel} builds require ${expectedScheme}//auth/callback.`,
      );
    }
    if (logout.href !== `${expectedScheme}//auth/logout-callback`) {
      throw new Error(
        `${channel} builds require ${expectedScheme}//auth/logout-callback.`,
      );
    }
    if (channel === "production") {
      if (issuer.protocol !== "https:" || isTestHostname(issuer.hostname)) {
        throw new Error(
          "Production builds require a non-test HTTPS OIDC issuer.",
        );
      }
    }
  }

  if (channel === "production" && env.VITE_OIDC_DESKTOP_PROXY_ORIGIN?.trim()) {
    throw new Error("Production builds must not include the local OIDC proxy.");
  }

  const businessOriginValue = env.VITE_BUSINESS_APP_ORIGIN?.trim();
  if (channel === "production" && businessOriginValue) {
    const businessOrigin = new URL(businessOriginValue);
    if (
      businessOrigin.protocol !== "https:" ||
      isTestHostname(businessOrigin.hostname)
    ) {
      throw new Error(
        "Production builds require a non-test HTTPS Business origin.",
      );
    }
  }
}

function withBusinessDockConfig(args) {
  if (!new Set(["build", "dev"]).has(args[0])) {
    return args;
  }
  const tauriConfigPath = new URL(
    "../src-tauri/tauri.conf.json",
    import.meta.url,
  );
  const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
  const mode = args[0] === "build" ? "production" : "development";
  const viteEnv = loadEnv(mode, process.cwd(), "VITE_");
  const channel = desktopBuildChannel(args);
  const buildEnv = { ...viteEnv, ...process.env };
  validateDesktopBuildEnvironment(channel, buildEnv);
  process.env.VITE_DESKTOP_BUILD_CHANNEL = channel;
  const csp = buildBusinessDockCsp(
    tauriConfig.app.security.csp,
    buildEnv.VITE_BUSINESS_APP_ORIGIN,
  );
  const override = JSON.stringify({ app: { security: { csp } } });
  const nextArgs = [...args];
  const separatorIndex = nextArgs.indexOf("--");
  nextArgs.splice(
    separatorIndex === -1 ? nextArgs.length : separatorIndex,
    0,
    "--config",
    override,
  );
  return nextArgs;
}

async function main() {
  const cli = require("@tauri-apps/cli");
  try {
    await cli.run(withBusinessDockConfig(process.argv.slice(2)), "pnpm tauri");
  } catch (error) {
    cli.logError(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  await main();
}
