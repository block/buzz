import * as React from "react";
import { useCommunities } from "../useCommunities";
import { COMMUNITY_NAME_REFRESH_EVENT } from "../useCommunityNames";
import {
  fetchCommunityProfile,
  setCommunityName,
} from "@/shared/api/communityProfile";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import type { Community } from "../types";

export function CommunityNameSettings({
  community,
  canRename,
}: {
  community: Community;
  canRename: boolean;
}) {
  const { activeCommunity } = useCommunities();
  const [name, setName] = React.useState(community.canonicalName ?? "");
  const [supported, setSupported] = React.useState<boolean | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState(false);
  const mounted = React.useRef(true);
  const activeRef = React.useRef(activeCommunity?.relayUrl);
  activeRef.current = activeCommunity?.relayUrl;
  React.useEffect(() => {
    let cancelled = false;
    mounted.current = true;
    void fetchCommunityProfile(community.relayUrl)
      .then((profile) => {
        if (cancelled) return;
        setSupported(profile !== null);
        if (profile) setName(profile.name ?? "");
      })
      .catch(() => {
        if (!cancelled)
          setError(
            "Couldn’t load the shared name. Reopen this editor to retry.",
          );
      });
    return () => {
      cancelled = true;
      mounted.current = false;
    };
  }, [community.relayUrl]);
  const save = async () => {
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      if (activeRef.current !== community.relayUrl)
        throw new Error("Switch to this community before renaming it.");
      await setCommunityName(
        name,
        community.relayUrl,
        () => mounted.current && activeRef.current === community.relayUrl,
      );
      window.dispatchEvent(new Event(COMMUNITY_NAME_REFRESH_EVENT));
      setSaved(true);
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : "Couldn’t save the shared name. Try again.",
      );
    } finally {
      setSaving(false);
    }
  };
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-sm font-medium" htmlFor="shared-community-name">
        Community name
      </label>
      <div className="flex gap-2">
        <Input
          id="shared-community-name"
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            setSaved(false);
          }}
          disabled={!canRename || !supported || saving}
          placeholder="Not set"
          aria-describedby="shared-community-name-help"
        />
        {canRename && supported ? (
          <Button
            type="button"
            disabled={
              saving ||
              !name.trim() ||
              new TextEncoder().encode(name.trim()).length > 256
            }
            onClick={() => void save()}
          >
            {saving ? "Saving…" : "Save name"}
          </Button>
        ) : null}
      </div>
      <p
        id="shared-community-name-help"
        className="text-xs text-muted-foreground"
      >
        {supported === false
          ? "Upgrade this relay to share a community name across devices."
          : "Set by an owner or admin. Shared across devices and publicly visible in relay information."}
      </p>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
      {saved ? (
        <p role="status" className="text-xs text-muted-foreground">
          Community name saved.
        </p>
      ) : null}
    </div>
  );
}
