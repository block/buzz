import { randomUUID } from "node:crypto";

/** A launch owns one channel ID even when creation or membership needs retrying. */
export interface Launch {
  mode: "new" | "join";
  channelId: string;
  agent?: string;
}

/** Validate before starting a host or making any relay writes. */
export function parseLaunch(args: string[], agent?: string): Launch {
  if (agent && !/^[0-9a-f]{64}$/i.test(agent))
    throw new Error("BUZZ_AGENT_PUBKEY must be a 64-character hex public key.");
  if (args.length === 0)
    return {
      mode: "new",
      channelId: randomUUID(),
      agent: agent?.toLowerCase(),
    };
  if (
    args.length === 2 &&
    args[0] === "join" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
      args[1],
    )
  )
    return {
      mode: "join",
      channelId: args[1].toLowerCase(),
      agent: agent?.toLowerCase(),
    };
  throw new Error(
    "Usage: buzz [join <channel-uuid>]. Use --demo for an offline preview.",
  );
}
