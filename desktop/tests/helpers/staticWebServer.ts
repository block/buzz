import { spawnSync } from "node:child_process";

const PORT = 4173;

/**
 * `python3` is the right name on Linux and macOS. On Windows it is normally
 * the Microsoft Store app-execution alias, which prints an install hint and
 * exits 9009 even when a real interpreter is installed as `python`. Playwright
 * then reports only `Process from config.webServer was not able to start.
 * Exit code: 9009`, so the suite cannot be run at all on a stock Windows box.
 *
 * Probe the candidates rather than guessing. `-c ""` is a no-op that a real
 * interpreter accepts and the alias stub rejects.
 */
function resolvePythonCommand() {
  const candidates =
    process.platform === "win32"
      ? ["python", "python3"]
      : ["python3", "python"];

  for (const candidate of candidates) {
    const probe = spawnSync(candidate, ["-c", ""], { stdio: "ignore" });
    if (!probe.error && probe.status === 0) {
      return candidate;
    }
  }

  // Nothing answered. Keep the historical name so the failure still points at
  // the missing interpreter instead of at this helper.
  return "python3";
}

export function staticWebServer(options: { reuseExistingServer: boolean }) {
  return {
    command: `${resolvePythonCommand()} -m http.server ${PORT} -d dist`,
    cwd: ".",
    reuseExistingServer: options.reuseExistingServer,
    url: `http://127.0.0.1:${PORT}`,
  };
}
