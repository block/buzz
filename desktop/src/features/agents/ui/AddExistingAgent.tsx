import { useId, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useManagedAgentsQuery } from "../hooks";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type Scope = { owner: string; community: string };

/** Explicit local-only identity import; never the snapshot mint path. */
export function AddExistingAgent() {
  const owner = useIdentityQuery().data?.pubkey;
  const { activeCommunity } = useCommunities();
  const community = activeCommunity?.relayUrl
    .trim()
    .replace(/^http/, "ws")
    .replace(/\/+$/, "");
  const { refetch } = useManagedAgentsQuery();
  if (!owner || !community) return null;
  return (
    <ExistingAgentDialog
      key={`${owner}:${community}`}
      scope={{ owner, community }}
      onSaved={() => {
        void refetch();
      }}
    />
  );
}

export function ExistingAgentDialog({
  scope,
  onSaved,
}: {
  scope: Scope;
  onSaved: () => void;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [pubkey, setPubkey] = useState("");
  const [privateKey, setPrivateKey] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);
  const submitting = useRef(false);
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        Add existing agent
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (submitting.current) return;
          setOpen(next);
          setPrivateKey("");
          setError(false);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add existing agent</DialogTitle>
            <DialogDescription>
              Keep the same agent identity and linked persona. The persona must
              already be available here. Nothing starts automatically. The
              supplied key stays on this computer, in its owner-only local agent
              file (not the OS keyring). Adding it again repairs a missing key
              without changing runtime settings.
            </DialogDescription>
          </DialogHeader>
          <form
            className="space-y-4"
            onSubmit={async (event) => {
              event.preventDefault();
              if (submitting.current) return;
              submitting.current = true;
              setPending(true);
              setError(false);
              const secret = privateKey;
              setPrivateKey("");
              try {
                await invoke("add_existing_agent", {
                  input: {
                    ...scope,
                    pubkey: pubkey.trim(),
                    privateKey: secret,
                  },
                });
                setOpen(false);
                onSaved();
              } catch {
                // Raw IPC exceptions may contain request data. Never display/log them.
                setError(true);
              } finally {
                submitting.current = false;
                setPending(false);
              }
            }}
          >
            <label htmlFor={`${id}-public`} className="block space-y-1">
              <span>Existing agent public key (hex)</span>
              <Input
                id={`${id}-public`}
                value={pubkey}
                onChange={(e) => setPubkey(e.target.value)}
                disabled={pending}
                required
                autoComplete="off"
              />
            </label>
            <label htmlFor={`${id}-secret`} className="block space-y-1">
              <span>Agent private key (nsec or hex)</span>
              <Input
                id={`${id}-secret`}
                type="password"
                value={privateKey}
                onChange={(e) => setPrivateKey(e.target.value)}
                disabled={pending}
                required
                autoComplete="off"
                spellCheck={false}
                autoCorrect="off"
                autoCapitalize="none"
              />
            </label>
            {error && (
              <p role="alert">
                Could not add this identity. Check the key, ownership, linked
                persona and community, then enter the key again to retry.
              </p>
            )}
            <Button
              type="submit"
              disabled={pending || !pubkey.trim() || !privateKey.trim()}
            >
              {pending ? "Adding…" : "Add existing agent"}
            </Button>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}
