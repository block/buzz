import { useState, useSyncExternalStore } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relayClient } from "@/shared/api/relayClient";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";

export function RelayEntryGate({
  relayUrl,
  communities,
  onSwitch,
  onRetry,
  onAddressChanged,
}: {
  relayUrl: string;
  communities: { id: string; name: string; relayUrl: string }[];
  onSwitch: (id: string) => void;
  onRetry: () => void;
  onAddressChanged: () => void;
}) {
  const entry = relayClient.entryRequirements;
  const state = useSyncExternalStore(entry.subscribe, entry.getSnapshot);
  const [editing, setEditing] = useState(false);
  const [openError, setOpenError] = useState(false);
  const visible = state.relayUrl === relayUrl && state.verdict === "required";
  return (
    <>
      <Dialog open={visible && !editing}>
        <DialogContent
          showCloseButton={false}
          onEscapeKeyDown={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
          data-testid="relay-entry-guidance"
        >
          <DialogTitle>Upgrade Buzz to join this community</DialogTitle>
          <DialogDescription>
            This community requires sign-in supported by a newer Buzz app. Your
            keys and saved communities are still here.
          </DialogDescription>
          <p className="break-all text-sm text-muted-foreground">{relayUrl}</p>
          <p className="text-sm">
            Download the latest Buzz client at buzz.xyz to continue.
          </p>
          <Button
            onClick={() => {
              setOpenError(false);
              void openUrl("https://buzz.xyz").catch(() => setOpenError(true));
            }}
          >
            Download latest Buzz
          </Button>
          {openError ? (
            <p role="alert" className="text-sm">
              Could not open the download page. Open https://buzz.xyz in your
              browser to download the latest Buzz client.
            </p>
          ) : null}
          <Button variant="outline" disabled={state.pending} onClick={onRetry}>
            {state.pending ? "Checking community…" : "Retry community check"}
          </Button>
          {communities
            .filter((community) => community.relayUrl !== relayUrl)
            .map((community) => (
              <Button
                variant="outline"
                key={community.id}
                onClick={() => onSwitch(community.id)}
              >
                Open {community.name}
              </Button>
            ))}
          <Button variant="ghost" onClick={() => setEditing(true)}>
            Change community address
          </Button>
        </DialogContent>
      </Dialog>
      {editing ? (
        <CommunityChangeOverlay
          onClose={() => setEditing(false)}
          onUpdated={(_name, updatedRelayUrl) => {
            if (updatedRelayUrl !== relayUrl) onAddressChanged();
            setEditing(false);
          }}
        />
      ) : null}
    </>
  );
}
