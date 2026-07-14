import * as React from "react";
import { toast } from "sonner";

import {
  useAddRelayMemberMutation,
  useRelayMembersQuery,
} from "@/features/community-members/hooks";
import type { RelayMemberRole } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

const PUBKEY_REGEX = /^[0-9a-f]{64}$/;

const ROLE_OPTIONS: Array<{ value: RelayMemberRole; label: string }> = [
  { value: "member", label: "Member" },
  { value: "admin", label: "Admin" },
];

export function AddMemberDialog({
  isOwner,
  open,
  onOpenChange,
}: {
  isOwner: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const addMutation = useAddRelayMemberMutation();
  const membersQuery = useRelayMembersQuery();
  const [pubkey, setPubkey] = React.useState("");
  const [role, setRole] = React.useState<RelayMemberRole>("member");

  const normalizedPubkey = pubkey.trim().toLowerCase();
  const isValidPubkey = PUBKEY_REGEX.test(normalizedPubkey);
  const isAlreadyMember =
    isValidPubkey &&
    !addMutation.isPending &&
    (membersQuery.data ?? []).some(
      (m) => m.pubkey.toLowerCase() === normalizedPubkey,
    );
  const canAdd = isValidPubkey && !isAlreadyMember && !addMutation.isPending;

  function reset() {
    setPubkey("");
    setRole("member");
    addMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      reset();
    }
    onOpenChange(next);
  }

  function handleAdd() {
    if (!canAdd) return;
    addMutation.mutate(
      { pubkey: normalizedPubkey, role },
      {
        onSuccess: () => {
          toast.success("Member added");
          handleOpenChange(false);
        },
      },
    );
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent
        className="max-w-md overflow-hidden p-0"
        data-testid="add-relay-member-dialog"
      >
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Add member</DialogTitle>
            <DialogDescription>
              Add a user to this relay by their public key.
            </DialogDescription>
          </DialogHeader>

          <form
            className="contents"
            onSubmit={(e) => {
              e.preventDefault();
              handleAdd();
            }}
          >
            <div className="flex-1 overflow-y-auto px-6 py-4">
              <div className="space-y-4">
                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="member-pubkey"
                  >
                    Public key
                  </label>
                  <Input
                    autoCapitalize="none"
                    autoCorrect="off"
                    data-testid="member-pubkey-input"
                    id="member-pubkey"
                    maxLength={64}
                    onChange={(e) => setPubkey(e.target.value)}
                    placeholder="64-character hex pubkey"
                    spellCheck={false}
                    value={pubkey}
                  />
                  {pubkey.trim().length > 0 && !isValidPubkey ? (
                    <p className="text-xs text-destructive">
                      Must be exactly 64 lowercase hex characters.
                    </p>
                  ) : null}
                  {isAlreadyMember ? (
                    <p className="text-xs text-destructive">
                      This pubkey is already a relay member.
                    </p>
                  ) : null}
                </div>

                <div className="space-y-1.5">
                  <p className="text-sm font-medium">Role</p>
                  <div className="flex gap-2">
                    {ROLE_OPTIONS.filter(
                      (opt) => isOwner || opt.value === "member",
                    ).map((opt) => (
                      <button
                        aria-pressed={role === opt.value}
                        className={cn(
                          "rounded-lg border px-3 py-1.5 text-sm transition-colors",
                          role === opt.value
                            ? "border-primary bg-primary/10 text-foreground"
                            : "border-border/60 text-muted-foreground hover:bg-accent",
                        )}
                        data-testid={`member-role-${opt.value}`}
                        key={opt.value}
                        onClick={() => setRole(opt.value)}
                        type="button"
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                </div>

                {addMutation.error instanceof Error ? (
                  <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                    {addMutation.error.message}
                  </p>
                ) : null}
              </div>
            </div>

            <div className="flex justify-end gap-2 border-t border-border/60 bg-background/95 px-6 py-4">
              <Button
                data-testid="cancel-add-member"
                onClick={() => handleOpenChange(false)}
                size="sm"
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
              <Button
                data-testid="confirm-add-member"
                disabled={!canAdd}
                size="sm"
                type="submit"
              >
                {addMutation.isPending ? "Adding..." : "Add member"}
              </Button>
            </div>
          </form>
        </div>
      </DialogContent>
    </Dialog>
  );
}
