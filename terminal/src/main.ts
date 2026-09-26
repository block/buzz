import { fileURLToPath } from "node:url";
import { ProcessTerminal } from "@earendil-works/pi-tui";
import { TerminalApp } from "./app.ts";
import { DemoTransport } from "./demo.ts";
import { type Launch, parseLaunch } from "./launch.ts";
import { AgentPreferences } from "./preferences.ts";
import { safeText } from "./theme.ts";
import { HostTransport } from "./transport.ts";

const args = process.argv.slice(2);
if (args.includes("--help")) {
  console.log(`Buzz terminal — a dedicated, multi-agent relay client

  buzz                              Create a private channel and start typing
  buzz join <channel-uuid>          Open an existing channel; never creates one
  pnpm --dir terminal start          Same new-channel launch from source
  pnpm --dir terminal demo           Offline UI preview; no relay writes

Build the Rust host first with just terminal-build.
Optional BUZZ_TERMINAL_HOST overrides the executable path.
BUZZ_AGENT_PUBKEY selects your default agent by exact hex key.
Ctrl+K conversations · Ctrl+R recipient · Ctrl+G commands · Ctrl+Q detach
Drafts are local to this terminal session. Agents run independently.`);
} else if (!process.stdin.isTTY || !process.stdout.isTTY) {
  console.error(
    "Buzz needs an interactive terminal. Use --help for connection instructions.",
  );
  process.exitCode = 1;
} else {
  const demo = args.length === 1 && args[0] === "--demo";
  let launch: Launch | undefined;
  try {
    launch = demo
      ? undefined
      : parseLaunch(args, process.env.BUZZ_AGENT_PUBKEY);
  } catch (error) {
    console.error(error instanceof Error ? error.message : "Invalid arguments");
    process.exit(1);
  }
  if (demo) {
    delete process.env.BUZZ_PRIVATE_KEY;
    delete process.env.BUZZ_AUTH_TAG;
  }
  const host =
    process.env.BUZZ_TERMINAL_HOST ||
    fileURLToPath(
      new URL("../../target/debug/buzz-terminal-host", import.meta.url),
    );
  const app = new TerminalApp(
    new ProcessTerminal(),
    demo ? new DemoTransport() : new HostTransport(host),
    () => {
      console.log("Detached from Buzz. Your agents keep working.");
      if (launch) console.log(`Resume with buzz join ${launch.channelId}`);
    },
    launch,
    demo ? undefined : new AgentPreferences(),
  );
  const fail = (error: unknown) => {
    app.stop();
    console.error(
      safeText(
        error instanceof Error ? error.message : "Buzz stopped unexpectedly.",
      ),
    );
    process.exitCode = 1;
  };
  process.once("SIGTERM", () => app.stop());
  process.once("SIGINT", () => app.stop());
  process.once("uncaughtException", fail);
  process.once("unhandledRejection", fail);
  try {
    app.start();
  } catch (error) {
    fail(error);
  }
}
