// One RemoteTerminalHub per page load. The Buzz term page registers it as the terminal backend when it mounts and unregisters
// when it unmounts, but the hub (and its signed connection and live sessions) survives, so leaving and coming back to Buzz term
// shows the same sessions instead of asking the signer again.
import type { RemoteInfo } from "./protocol";
import type { SignJson } from "./remoteClient";
import { RemoteTerminalHub } from "./remoteTerminal";

let hub: RemoteTerminalHub | null = null;
let info: RemoteInfo | undefined;
let sign: SignJson | null = null;

export function getWebTermHub(): RemoteTerminalHub {
  hub ??= new RemoteTerminalHub(
    () => info,
    (e) => {
      if (!sign) throw new Error("signer not ready");
      return sign(e);
    },
  );
  return hub;
}

/** Keep the hub pointed at the desktop's latest heartbeat and the current signer (called on every render of the page). */
/** What the desktop last advertised for remote terminals (read-only; e.g. termHost "herdr"). */
export function getWebTermInfo(): RemoteInfo | undefined {
  return info;
}

export function configureWebTermHub(next: { info: RemoteInfo | undefined; sign: SignJson }) {
  info = next.info;
  sign = next.sign;
}

export function resetWebTermHubForTests() {
  hub?.dispose();
  hub = null;
  info = undefined;
  sign = null;
}
