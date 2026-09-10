import { accessSync, constants, realpathSync, statSync } from 'node:fs';
import { isAbsolute, join } from 'node:path';
/** Pinned Desktop definitions, not vendor capability or authentication claims.
 * 051c3a2 desktop/src-tauri/src/managed_agents/discovery/presets.rs:69–228.
 */
export const presets = [
  {
    "id": "pi",
    "label": "Pi",
    "command": "pi-acp",
    "args": [],
    "env": {},
    "installInstructionsUrl": "https://github.com/svkozak/pi-acp",
    "installHint": "Install Pi with npm install -g --ignore-scripts @earendil-works/pi-coding-agent. Install the Pi ACP adapter with npm install -g pi-acp.",
    "underlyingCli": "pi"
  },
  {
    "id": "devin",
    "label": "Devin",
    "command": "devin",
    "args": [
      "acp"
    ],
    "env": {},
    "installInstructionsUrl": "https://docs.devin.ai/cli",
    "installHint": "Buzz talks to Devin through the official Devin CLI's ACP mode (devin acp)."
  },
  {
    "id": "cursor",
    "label": "Cursor",
    "command": "cursor-agent",
    "args": [
      "acp"
    ],
    "env": {},
    "installInstructionsUrl": "https://cursor.com/downloads",
    "installHint": "Buzz talks to Cursor through the cursor-agent CLI's ACP mode."
  },
  {
    "id": "omp",
    "label": "Oh My Pi",
    "command": "omp",
    "args": [
      "acp"
    ],
    "env": {},
    "installInstructionsUrl": "https://omp.sh/",
    "installHint": "Buzz talks to Oh My Pi through its CLI's ACP mode (omp acp)."
  },
  {
    "id": "grok",
    "label": "Grok Build",
    "command": "grok",
    "args": [
      "agent",
      "--always-approve",
      "stdio"
    ],
    "env": {},
    "installInstructionsUrl": "https://build.x.ai/docs",
    "installHint": "Buzz talks to Grok Build through its CLI's agent stdio mode."
  },
  {
    "id": "opencode",
    "label": "OpenCode",
    "command": "opencode",
    "args": [
      "acp"
    ],
    "env": {},
    "installInstructionsUrl": "https://opencode.ai/docs",
    "installHint": "Buzz talks to OpenCode through its CLI's ACP mode (opencode acp)."
  },
  {
    "id": "kimi",
    "label": "Kimi Code",
    "command": "kimi",
    "args": [
      "acp"
    ],
    "env": {},
    "installInstructionsUrl": "https://kimi.ai/download",
    "installHint": "Buzz talks to Kimi Code through its CLI's ACP mode (kimi acp)."
  },
  {
    "id": "amp",
    "label": "Amp",
    "command": "amp-acp",
    "args": [],
    "env": {},
    "installInstructionsUrl": "https://github.com/tao12345666333/amp-acp",
    "installHint": "Buzz talks to the Amp CLI through the amp-acp adapter. Follow the setup guide to install the adapter so the amp-acp command is on your PATH.",
    "underlyingCli": "amp"
  },
  {
    "id": "hermes",
    "label": "Hermes Agent",
    "command": "hermes-acp",
    "args": [],
    "env": {},
    "installInstructionsUrl": "https://hermes-agent.nousresearch.com",
    "installHint": "Buzz talks to Hermes Agent through its hermes-acp command."
  },
  {
    "id": "openclaw",
    "label": "OpenClaw",
    "command": "openclaw",
    "args": [
      "acp"
    ],
    "env": {},
    "installInstructionsUrl": "https://docs.openclaw.ai/start/getting-started",
    "installHint": "Install the amp-acp npm adapter.",
    "underlyingCli": "amp"
  }
] as const;
/** Process-free discovery in explicit local PATH only; never a login shell. */
export function discoverPresets(path: string = process.env.PATH ?? '') {
  const directories = path.split(':').filter(isAbsolute).slice(0, 128);
  const resolve = (command: string): string | undefined => {
    for (const directory of directories) {
      const candidate = join(directory, command);
      try { if (!statSync(candidate).isFile()) continue; accessSync(candidate, constants.X_OK); return realpathSync(candidate); }
      catch { /* Missing/inaccessible candidate is diagnostic absence, not auth. */ }
    }
    return undefined;
  };
  return presets.map(preset => {
    const executable = resolve(preset.command);
    const cli = 'underlyingCli' in preset ? resolve(preset.underlyingCli) : undefined;
    const availability = 'underlyingCli' in preset ? executable ? cli ? 'available' : 'cli-missing' : cli ? 'adapter-missing' : 'not-installed' : executable ? 'available' : 'executable-missing';
    return { ...preset, executable, availability, authentication: 'unverified' as const,
      reason: 'Setup/diagnostic only: pinned preset source provides no exact actual-session model and native profile contract. NotApplicable auth is not authenticated.' };
  });
}
/** Local display only: no execution, install scripts or remote credentials. */
export function showPresets(): void {
  for (const p of discoverPresets()) console.log(`${p.id} | ${p.label} | ${p.availability} | ${p.command} ${JSON.stringify(p.args)} | env {}\n${p.installHint} ${p.installInstructionsUrl}\n${p.reason}`);
  console.log('Sign in/configure locally as the dedicated host service user, not Desktop HOME. No login or ACP process was launched.');
}
