import { composeSlashCommands } from "../lib/composeSlashCommands";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";
import { useChannelAdminCommands } from "./channelAdmin";
import { useMembershipCommands } from "./membership";
import { useNavigationCommands } from "./navigation";
import { usePresenceCommands } from "./presence";
import { useRemindCommands } from "./remind";
import { useTextCommands } from "./text";

/** Every command usable in this composer; empty when commands are off. */
export function useSlashCommandRegistry(
  environment: SlashCommandEnvironment,
): SlashCommandDefinition[] {
  const presence = usePresenceCommands(environment);
  const membership = useMembershipCommands(environment);
  const channelAdmin = useChannelAdminCommands(environment);
  const navigation = useNavigationCommands(environment);
  const remind = useRemindCommands(environment);
  const text = useTextCommands(environment);
  return composeSlashCommands(
    [presence, membership, channelAdmin, navigation, remind, text],
    { enabled: environment.enabled },
  );
}
