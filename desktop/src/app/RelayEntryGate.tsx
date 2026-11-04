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
}: {
  relayUrl: string;
  communities: { id: string; name: string; relayUrl: string }[];
  onSwitch: (id: string) => void;
  onRetry: () => void;
}) {
  const entry = relayClient.entryRequirements;
  const state = useSyncExternalStore(entry.subscribe, entry.getSnapshot);
  const [editing, setEditing] = useState(false);
  const [openError, setOpenError] = useState(false);
  const visible = state.relayUrl === relayUrl && state.verdict !== null;
  const required = state.verdict === "required";
  return (
    <>
      <Dialog open={visible && !editing}>
        <DialogContent
          showCloseButton={false}
          onEscapeKeyDown={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
          data-testid="relay-entry-guidance"
        >
          <DialogTitle>
            {required
              ? "Upgrade Buzz to join this community"
              : "Could not check this community"}
          </DialogTitle>
          <DialogDescription>
            {required
              ? "This community requires sign-in supported by a newer Buzz app. Your keys and saved communities are still here."
              : "The community's sign-in requirements could not be checked. Try again when the community is reachable."}
          </DialogDescription>
          <p className="break-all text-sm text-muted-foreground">{relayUrl}</p>
          {required && !state.upgradeUrl ? (
            <p className="text-sm">
              Ask your community administrator for a Buzz installer that
              supports this community and your computer.
            </p>
          ) : null}
          {required && state.upgradeUrl ? (
            <Button
              onClick={() => {
                setOpenError(false);
                if (!state.upgradeUrl) return;
                void openUrl(state.upgradeUrl).catch(() => setOpenError(true));
              }}
            >
              Download newer Buzz
            </Button>
          ) : null}
          {openError ? (
            <p role="alert" className="text-sm">
              Could not open the download. Try again or ask your community
              administrator.
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
          onUpdated={() => setEditing(false)}
        />
      ) : null}
    </>
  );
}
