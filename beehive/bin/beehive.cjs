#!/usr/bin/env node
/* Beehive standalone command launcher.
 *
 * Runs the package CLI (src/cli.ts) from this checkout under the same Node.js,
 * resolving everything from this file's real location so it works from any
 * directory, through a user-installed symlink, and from paths containing
 * spaces. One-time reversible user install (no shell profile or system
 * change), for example into a user bin directory already on PATH:
 *
 *   ln -s /absolute/path/to/beehive/bin/beehive.cjs ~/.local/bin/beehive
 *
 * Plain CommonJS on purpose: the Node version gate must be checkable before
 * any TypeScript loading (the CLI needs Node's type stripping). */
'use strict';
const { spawnSync } = require('node:child_process');
const { join } = require('node:path');

/** Mirrors package.json "engines": Node.js >= 22.18.0. */
function meetsMinimumNode(version, minimum) {
  const [major, minor] = String(version).split('.').map(Number);
  const [minimumMajor, minimumMinor] = String(minimum).split('.').map(Number);
  return major > minimumMajor || (major === minimumMajor && minor >= minimumMinor);
}

function run() {
  if (!meetsMinimumNode(process.versions.node, '22.18.0')) {
    console.error(`beehive: Node.js 22.18.0 or newer is required; this launcher ran under Node ${process.versions.node}. Install a current Node.js (https://nodejs.org), make sure it is on PATH, and run beehive again.`);
    process.exit(1);
  }
  const cli = join(__dirname, '..', 'src', process.argv.length === 2 ? 'manager-entry.ts' : 'cli.ts');
  const packagedNode = join(__dirname, '..', 'runtime', 'node');
  const node = require('node:fs').existsSync(packagedNode) ? packagedNode : process.execPath;
  const child = spawnSync(node, [cli, ...process.argv.slice(2)], { stdio: 'inherit' });
  if (child.error) {
    console.error(`beehive: cannot start the Beehive CLI at ${cli}: ${child.error.message}`);
    process.exit(1);
  }
  if (child.status !== null) process.exit(child.status);
  if (child.signal) process.kill(process.pid, child.signal); // Propagate Ctrl-C/termination as the same signal.
  process.exit(1);
}

if (require.main === module) run();

module.exports = { meetsMinimumNode };
