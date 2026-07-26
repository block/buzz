import { ServerCog } from "lucide-react";
import { toast } from "sonner";

import type { ManagedAgent } from "@/shared/api/types";
import {
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/shared/ui/dropdown-menu";
import { useSpawnerDirectory } from "../spawnerDirectoryStore";
import { useServerAgents } from "../useServerAgents";
import { spawnerLabel } from "./ServerAgentsSection";

/**
 * "Move to server" for an agent that already exists on this Mac.
 *
 * Deliberately *not* the Deploy menu: deploying a persona mints a new key and
 * leaves the local agent alone, while this keeps the agent's existing key and
 * retires the local copy. Same agent, different machine — which is the only way
 * its channel membership, profile, DMs, and NIP-AE memory survive the move.
 *
 * Renders nothing when there is no local agent to move or no spawner to move it
 * to; a disabled item for an action that cannot exist yet is just noise.
 */
export function MoveAgentToServerMenu({
  agent,
  disabled,
}: {
  agent: ManagedAgent | undefined;
  disabled: boolean;
}) {
  const { spawners, relocate, isPending } = useServerAgents();
  const directory = useSpawnerDirectory();

  if (!agent || spawners.length === 0) return null;

  const handleMove = async (spawnerPubkey: string) => {
    try {
      await relocate(agent, spawnerPubkey);
      toast.success(
        `Moving ${agent.name} to ${spawnerLabel(spawnerPubkey, directory)}. ` +
          `Approve the key when prompted — it keeps its identity and stops running on this Mac.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Failed to move ${agent.name} to the server.`,
      );
    }
  };

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger disabled={disabled || isPending}>
        <ServerCog className="h-4 w-4" />
        Move to server
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent>
        {spawners.map((spawnerPubkey) => (
          <DropdownMenuItem
            disabled={disabled || isPending}
            key={spawnerPubkey}
            onSelect={() => void handleMove(spawnerPubkey)}
          >
            {spawnerLabel(spawnerPubkey, directory)}
          </DropdownMenuItem>
        ))}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
