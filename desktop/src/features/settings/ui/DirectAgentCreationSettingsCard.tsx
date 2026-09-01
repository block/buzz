import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  setDirectAgentCreationGrant,
  useDirectAgentCreationGrants,
} from "@/features/agents/directAgentCreationGrant";
import { Switch } from "@/shared/ui/switch";
import { useIdentityQuery } from "@/shared/api/hooks";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

/** One-time, revocable grants for owned agents to create owner-only agents. */
export function DirectAgentCreationSettingsCard() {
  const managedAgents = useManagedAgentsQuery();
  const ownerPubkey = useIdentityQuery().data?.pubkey ?? "";
  const grants = useDirectAgentCreationGrants(ownerPubkey);
  const granted = new Set(grants);

  if ((managedAgents.data?.length ?? 0) === 0) return null;

  return (
    <SettingsOptionGroup title="Agent permissions">
      <div className="px-4 py-3 text-sm text-muted-foreground">
        A granted agent can create and start new owner-only agents using your
        saved defaults, then add them to the channel where it was asked. The
        action is reported in that channel. Revoke access here at any time.
      </div>
      {managedAgents.data?.map((agent) => {
        const id = `settings-direct-agent-create-${agent.pubkey}`;
        return (
          <SettingsOptionRow data-testid={id} key={agent.pubkey}>
            <div className="min-w-0">
              <label className="font-medium text-foreground" htmlFor={id}>
                {agent.name}
              </label>
              <p className="mt-0.5 text-sm text-muted-foreground/70">
                Allow direct agent creation
              </p>
            </div>
            <Switch
              aria-label={`Allow ${agent.name} to create agents`}
              checked={granted.has(agent.pubkey.toLowerCase())}
              id={id}
              onCheckedChange={(enabled) =>
                setDirectAgentCreationGrant(ownerPubkey, agent.pubkey, enabled)
              }
            />
          </SettingsOptionRow>
        );
      })}
    </SettingsOptionGroup>
  );
}
