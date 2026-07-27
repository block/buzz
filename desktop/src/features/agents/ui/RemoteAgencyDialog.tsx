import * as React from "react";
import { ExternalLink, LoaderCircle, Network, ShieldCheck } from "lucide-react";

import { useChannelsQuery } from "@/features/channels/hooks";
import { useCreateManagedAgentMutation } from "@/features/agents/hooks";
import {
  bindingFromRemoteAgencyProxies,
  buildRemoteAgencyManagedAgentInput,
  findRemoteAgencyBinding,
  findRemoteAgencyProxy,
} from "@/features/agents/lib/remoteAgencyJoin";
import {
  addChannelMembers,
  listRemoteAgencies,
  previewRemoteAgency,
  saveRemoteAgencyBinding,
  storeRemoteAgencyBearerToken,
  updateManagedAgent,
} from "@/shared/api/tauri";
import {
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import type { RemoteAgencyDescriptor } from "@/shared/api/remoteAgencyTypes";
import type { Channel } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

type RemoteAgencyDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onBindingChange?: () => void;
};

function targetChannels(channels: Channel[] | undefined) {
  return (channels ?? []).filter(
    (channel) => channel.channelType !== "dm" && !channel.archivedAt,
  );
}

export function RemoteAgencyDialog({
  open,
  onOpenChange,
  onBindingChange,
}: RemoteAgencyDialogProps) {
  const channelsQuery = useChannelsQuery({ enabled: open });
  const createMutation = useCreateManagedAgentMutation();
  const [sourceUrl, setSourceUrl] = React.useState("");
  const [descriptor, setDescriptor] =
    React.useState<RemoteAgencyDescriptor | null>(null);
  const [selectedAgentIds, setSelectedAgentIds] = React.useState<string[]>([]);
  const [selectedSpaceIds, setSelectedSpaceIds] = React.useState<string[]>([]);
  const [channelId, setChannelId] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [credentialMessage, setCredentialMessage] = React.useState<
    string | null
  >(null);
  const [isPreviewing, setIsPreviewing] = React.useState(false);
  const [isJoining, setIsJoining] = React.useState(false);
  const bearerTokenRef = React.useRef<HTMLInputElement>(null);

  const channels = React.useMemo(
    () => targetChannels(channelsQuery.data),
    [channelsQuery.data],
  );

  React.useEffect(() => {
    if (open && !channelId && channels.length > 0) {
      setChannelId(channels[0].id);
    }
  }, [channelId, channels, open]);

  function reset() {
    setSourceUrl("");
    setDescriptor(null);
    setSelectedAgentIds([]);
    setSelectedSpaceIds([]);
    setChannelId("");
    setError(null);
    setCredentialMessage(null);
    setIsPreviewing(false);
    setIsJoining(false);
    createMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) reset();
    onOpenChange(next);
  }

  async function handlePreview(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setCredentialMessage(null);
    setIsPreviewing(true);
    try {
      const next = await previewRemoteAgency(sourceUrl.trim());
      setDescriptor(next);
      const joinableAgent = next.agents.find(
        (agent) => agent.recordUrl && agent.a2aEndpoint,
      );
      setSelectedAgentIds(joinableAgent ? [joinableAgent.id] : []);
      setSelectedSpaceIds(next.spaces.length > 0 ? [next.spaces[0].id] : []);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsPreviewing(false);
    }
  }

  async function handleJoin() {
    if (!descriptor || selectedAgentIds.length === 0 || !channelId) return;
    setError(null);
    setIsJoining(true);
    try {
      const joinedPubkeys: string[] = [];
      const failures: string[] = [];
      const currentBindings = await listRemoteAgencies();
      const existingBinding = findRemoteAgencyBinding(
        currentBindings,
        descriptor,
      );
      const proxies = [...(existingBinding?.proxies ?? [])];
      const bearerToken = bearerTokenRef.current?.value ?? "";
      for (const agentId of selectedAgentIds) {
        const remote = descriptor.agents.find((agent) => agent.id === agentId);
        if (!remote) continue;
        if (!remote.recordUrl || !remote.a2aEndpoint) {
          throw new Error(
            `${remote.name} no longer advertises a public OASF Agent Record and reviewed A2A endpoint`,
          );
        }
        const selectedSpaceId = selectedSpaceIds[0];
        if (bearerToken) {
          await storeRemoteAgencyBearerToken({
            recordUrl: remote.recordUrl,
            endpoint: remote.a2aEndpoint,
            token: bearerToken,
          });
        }
        const existingProxy = findRemoteAgencyProxy(
          proxies,
          remote.id,
          channelId,
          selectedSpaceId ?? null,
        );
        if (existingProxy) {
          joinedPubkeys.push(existingProxy.pubkey);
          try {
            const desired = buildRemoteAgencyManagedAgentInput(
              descriptor,
              remote,
              channelId,
              selectedSpaceId ?? null,
            );
            await stopManagedAgent(existingProxy.pubkey);
            await updateManagedAgent({
              pubkey: existingProxy.pubkey,
              name: desired.name,
              acpCommand: desired.acpCommand,
              agentCommand: desired.agentCommand,
              harnessOverride: desired.harnessOverride,
              agentArgs: desired.agentArgs,
              envVars: desired.envVars,
              parallelism: desired.parallelism,
            });
            const existingProxyIndex = proxies.indexOf(existingProxy);
            proxies[existingProxyIndex] = {
              ...existingProxy,
              recordUrl: remote.recordUrl,
              recordRevision: remote.recordRevision,
            };
            await saveRemoteAgencyBinding(
              bindingFromRemoteAgencyProxies(
                descriptor,
                proxies,
                existingBinding?.joinedAt,
              ),
            );
            await startManagedAgent(existingProxy.pubkey);
          } catch (cause) {
            failures.push(
              `${remote.name}: ${
                cause instanceof Error ? cause.message : String(cause)
              }`,
            );
          }
          continue;
        }
        const created = await createMutation.mutateAsync(
          buildRemoteAgencyManagedAgentInput(
            descriptor,
            remote,
            channelId,
            selectedSpaceId ?? null,
          ),
        );
        proxies.push({
          agentId: remote.id,
          pubkey: created.agent.pubkey,
          channelId,
          spaceId: selectedSpaceId ?? null,
          recordUrl: remote.recordUrl,
          recordRevision: remote.recordRevision,
        });
        await saveRemoteAgencyBinding(
          bindingFromRemoteAgencyProxies(
            descriptor,
            proxies,
            existingBinding?.joinedAt,
          ),
        );
        if (created.spawnError) {
          failures.push(
            `${remote.name}: proxy configured but not started: ${created.spawnError}`,
          );
        }
        joinedPubkeys.push(created.agent.pubkey);
      }
      const membership = await addChannelMembers({
        channelId,
        pubkeys: [...new Set(joinedPubkeys)],
        role: "bot",
      });
      failures.push(
        ...membership.errors.map(
          ({ pubkey, error: membershipError }) =>
            `${truncatePubkey(pubkey)}: channel membership failed: ${membershipError}`,
        ),
      );
      await saveRemoteAgencyBinding(
        bindingFromRemoteAgencyProxies(
          descriptor,
          proxies,
          existingBinding?.joinedAt,
        ),
      );
      if (bearerTokenRef.current) bearerTokenRef.current.value = "";
      onBindingChange?.();
      if (failures.length > 0) {
        setError(
          `The proxy identities were saved and can be retried without duplication. ${failures.join(
            " ",
          )}`,
        );
      } else {
        handleOpenChange(false);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsJoining(false);
    }
  }

  async function handleClearStoredCredential() {
    if (!descriptor) return;
    setError(null);
    setCredentialMessage(null);
    const selectedEndpoints = descriptor.agents.flatMap((agent) => {
      if (
        !selectedAgentIds.includes(agent.id) ||
        !agent.recordUrl ||
        !agent.a2aEndpoint
      ) {
        return [];
      }
      return [
        { endpoint: agent.a2aEndpoint, recordUrl: agent.recordUrl } as const,
      ];
    });
    try {
      await Promise.all(
        selectedEndpoints.map(({ endpoint, recordUrl }) =>
          storeRemoteAgencyBearerToken({
            endpoint,
            recordUrl,
            token: "",
          }),
        ),
      );
      if (bearerTokenRef.current) bearerTokenRef.current.value = "";
      setCredentialMessage(
        `Cleared stored credentials for ${selectedEndpoints.length} selected endpoint${
          selectedEndpoints.length === 1 ? "" : "s"
        }.`,
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-h-[min(760px,calc(100vh-2rem))] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Network className="size-4" />
            Add Remote Agency
          </DialogTitle>
          <DialogDescription>
            Preview an agency manifest, then join selected agents to a Buzz
            channel through local proxy identities.
          </DialogDescription>
        </DialogHeader>

        {!descriptor ? (
          <form className="space-y-3" onSubmit={handlePreview}>
            <Input
              aria-label="Remote Agency URL"
              onChange={(event) => setSourceUrl(event.target.value)}
              placeholder="https://agency.example/.well-known/agency.json"
              required
              type="url"
              value={sourceUrl}
            />
            <p className="text-xs text-muted-foreground">
              HTTPS is required. HTTP is allowed only for localhost development.
              Buzz imports public identity and capability metadata only.
            </p>
            <Button disabled={isPreviewing || !sourceUrl.trim()} type="submit">
              {isPreviewing ? <LoaderCircle className="animate-spin" /> : null}
              Preview Agency
            </Button>
          </form>
        ) : (
          <div className="space-y-5">
            <div className="rounded-xl border border-border/60 bg-muted/20 p-4">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h3 className="font-medium">{descriptor.name}</h3>
                  <p className="mt-1 break-all text-xs text-muted-foreground">
                    {descriptor.sourceUrl}
                  </p>
                </div>
                <Badge variant="outline">Remote Agency</Badge>
              </div>
              {descriptor.description ? (
                <p className="mt-3 text-sm text-muted-foreground">
                  {descriptor.description}
                </p>
              ) : null}
              <div className="mt-3">
                <p className="mb-1.5 text-xs text-muted-foreground">
                  Declared by the agency manifest
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {[...descriptor.protocols, ...descriptor.capabilities]
                    .slice(0, 8)
                    .map((value) => (
                      <Badge key={value} variant="secondary">
                        {value}
                      </Badge>
                    ))}
                </div>
              </div>
            </div>

            <section className="space-y-2">
              <h3 className="text-sm font-medium">Agents to join</h3>
              {descriptor.agents.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No public agents were advertised.
                </p>
              ) : null}
              {descriptor.agents.map((agent) => (
                <div
                  className="flex items-start gap-3 rounded-lg border border-border/50 p-3"
                  key={agent.id}
                >
                  <Checkbox
                    aria-label={`Join ${agent.name}`}
                    checked={selectedAgentIds.includes(agent.id)}
                    disabled={!agent.recordUrl || !agent.a2aEndpoint}
                    onCheckedChange={(checked) =>
                      setSelectedAgentIds((current) =>
                        checked
                          ? [...new Set([...current, agent.id])]
                          : current.filter((id) => id !== agent.id),
                      )
                    }
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block font-medium">{agent.name}</span>
                    {agent.description ? (
                      <span className="mt-0.5 block text-xs text-muted-foreground">
                        {agent.description}
                      </span>
                    ) : null}
                    {agent.agentCardUrl ? (
                      <a
                        className="mt-1 inline-flex items-center gap-1 text-xs text-primary hover:underline"
                        href={agent.agentCardUrl}
                        rel="noreferrer"
                        target="_blank"
                      >
                        A2A Agent Card <ExternalLink className="size-3" />
                      </a>
                    ) : null}
                    {agent.recordUrl ? (
                      <a
                        className="mt-1 ml-3 inline-flex items-center gap-1 text-xs text-primary hover:underline"
                        href={agent.recordUrl}
                        rel="noreferrer"
                        target="_blank"
                      >
                        OASF Agent Record <ExternalLink className="size-3" />
                      </a>
                    ) : null}
                    {agent.a2aEndpoint ? (
                      <span className="mt-1 block text-xs text-muted-foreground">
                        A2A endpoint configured: {agent.a2aEndpoint}
                      </span>
                    ) : null}
                    {!agent.recordUrl || !agent.a2aEndpoint ? (
                      <span className="mt-1 block text-xs text-muted-foreground">
                        Missing public OASF Agent Record or A2A endpoint; this
                        agent is preview-only.
                      </span>
                    ) : null}
                  </span>
                </div>
              ))}
            </section>

            <section className="space-y-2">
              <h3 className="text-sm font-medium">Spaces and surfaces</h3>
              <p className="text-xs text-muted-foreground">
                Spaces and surfaces are advertised metadata in this release.
                Buzz can bind a proxy to a Space, but it does not install or
                render the advertised surfaces yet.
              </p>
              {descriptor.spaces.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No public Spaces were advertised.
                </p>
              ) : null}
              {descriptor.spaces.map((space) => (
                <div
                  className="block rounded-lg border border-border/50 p-3"
                  key={space.id}
                >
                  <span className="flex items-start gap-3">
                    <Checkbox
                      aria-label={`Use ${space.name} Space`}
                      checked={selectedSpaceIds.includes(space.id)}
                      onCheckedChange={(checked) =>
                        setSelectedSpaceIds(checked ? [space.id] : [])
                      }
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block font-medium">{space.name}</span>
                      {space.description ? (
                        <span className="block text-xs text-muted-foreground">
                          {space.description}
                        </span>
                      ) : null}
                    </span>
                  </span>
                  {space.surfaces.length > 0 ? (
                    <div className="ml-7 mt-2 flex flex-wrap gap-1.5">
                      {space.surfaces.map((surface) => (
                        <Badge key={surface.id} variant="outline">
                          {surface.name}
                          {surface.surfaceType
                            ? ` · ${surface.surfaceType}`
                            : ""}
                        </Badge>
                      ))}
                    </div>
                  ) : null}
                </div>
              ))}
            </section>

            <label className="block space-y-2 text-sm font-medium">
              Buzz channel
              <select
                className="flex h-9 w-full rounded-lg border border-input/40 bg-background px-3 text-sm"
                onChange={(event) => setChannelId(event.target.value)}
                value={channelId}
              >
                <option value="">Select a channel</option>
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    #{channel.name}
                  </option>
                ))}
              </select>
            </label>

            <div className="space-y-2">
              <label
                className="block text-sm font-medium"
                htmlFor="remote-agency-bearer-token"
              >
                A2A bearer token
              </label>
              <Input
                aria-describedby="remote-agency-bearer-token-help"
                autoComplete="off"
                id="remote-agency-bearer-token"
                placeholder="Optional; stored in Keychain per reviewed endpoint"
                ref={bearerTokenRef}
                type="password"
              />
              <div className="flex items-start justify-between gap-3">
                <span
                  className="block text-xs font-normal text-muted-foreground"
                  id="remote-agency-bearer-token-help"
                >
                  One token is applied to each selected endpoint for this join.
                  Leave it blank for public endpoints or to reuse a token
                  already stored on this machine.
                </span>
                <Button
                  className="h-auto shrink-0 px-0 py-0 text-xs"
                  disabled={
                    !descriptor.agents.some(
                      (agent) =>
                        selectedAgentIds.includes(agent.id) &&
                        agent.recordUrl &&
                        agent.a2aEndpoint,
                    )
                  }
                  onClick={() => void handleClearStoredCredential()}
                  type="button"
                  variant="link"
                >
                  Clear stored credential
                </Button>
              </div>
              {credentialMessage ? (
                <p className="text-xs text-muted-foreground" role="status">
                  {credentialMessage}
                </p>
              ) : null}
            </div>

            <div className="flex items-start gap-2 rounded-lg border border-border/50 bg-muted/20 p-3 text-xs text-muted-foreground">
              <ShieldCheck className="mt-0.5 size-4 shrink-0 text-primary" />
              <span>
                Buzz creates a local Nostr identity for each proxy. The remote
                runtime keeps its own keys, prompts, memory, tools, and signing
                authority. Endpoint credentials are stored in the OS Keychain
                and are injected only into the matching A2A adapter.
              </span>
            </div>

            {error ? (
              <p className="text-sm text-destructive" role="alert">
                {error}
              </p>
            ) : null}
            <div className="flex justify-between gap-2">
              <Button onClick={() => setDescriptor(null)} variant="outline">
                Back
              </Button>
              <Button
                disabled={
                  isJoining || selectedAgentIds.length === 0 || !channelId
                }
                onClick={() => void handleJoin()}
              >
                {isJoining ? <LoaderCircle className="animate-spin" /> : null}
                Join selected agents
              </Button>
            </div>
          </div>
        )}
        {error && !descriptor ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
