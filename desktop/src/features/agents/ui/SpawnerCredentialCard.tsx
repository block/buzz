import { Check, CircleAlert } from "lucide-react";
import React from "react";

import { sendSpawnerCredentialUpdate } from "@/shared/api/spawnerRelay";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { waitForSpawnerCredentialAck } from "../spawnerCredentialAcks";
import {
  submitSpawnerCredential,
  type CredentialSubmitResult,
} from "./spawnerCredentialSubmit";

/** How long to wait for the spawner's encrypted ack before reporting failure. */
const ACK_TIMEOUT_MS = 15_000;

type Status = { kind: "idle" } | { kind: "sending" } | CredentialSubmitResult;

/**
 * Write-only entry for the owner's Claude credential on one spawner.
 *
 * The token is sent straight to Rust for encryption and never stored on this
 * device — the spawner is the source of truth, which is why nothing here reads
 * a saved value back. Server agents on this spawner do not run until the owner
 * provisions a token (the spawner reports `needs_credential` on their status).
 */
export function SpawnerCredentialCard({
  spawnerPubkey,
  spawnerName,
}: {
  spawnerPubkey: string;
  spawnerName: string;
}) {
  const [value, setValue] = React.useState("");
  const [status, setStatus] = React.useState<Status>({ kind: "idle" });
  const inputId = React.useId();

  const submit = async (credential: string) => {
    setStatus({ kind: "sending" });
    const result = await submitSpawnerCredential(
      {
        send: sendSpawnerCredentialUpdate,
        waitForAck: waitForSpawnerCredentialAck,
      },
      spawnerPubkey,
      credential,
      ACK_TIMEOUT_MS,
    );
    setStatus(result);
    // Write-only: the field empties only after a confirmed save.
    if (result.kind === "saved" && !result.cleared) setValue("");
  };

  const sending = status.kind === "sending";

  return (
    <div
      className="space-y-2 rounded-md border border-border p-3"
      data-testid="spawner-credential-card"
    >
      <label className="text-sm font-medium" htmlFor={inputId}>
        Your Claude credential
      </label>
      <p className="text-2xs text-muted-foreground">
        Agents you run on {spawnerName} use your own token. Paste a Claude Code
        OAuth token (sk-ant-oat…) or an Anthropic API key. It is sent encrypted
        to the server and never stored on this device.
      </p>
      <div className="flex gap-2">
        <Input
          autoComplete="off"
          data-testid="spawner-credential-input"
          disabled={sending}
          id={inputId}
          onChange={(event) => setValue(event.target.value)}
          placeholder="sk-ant-…"
          type="password"
          value={value}
        />
        <Button
          disabled={sending || value.trim().length === 0}
          onClick={() => void submit(value.trim())}
          type="button"
          variant="outline"
        >
          Save
        </Button>
        <Button
          disabled={sending}
          onClick={() => void submit("")}
          type="button"
          variant="ghost"
        >
          Clear
        </Button>
      </div>
      {status.kind === "saved" ? (
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          <Check aria-hidden className="size-3" />
          {status.cleared
            ? "Credential cleared. Your agents here will stop."
            : "Provisioned. Your agents here are restarting with it."}
        </p>
      ) : null}
      {status.kind === "error" ? (
        <p className="flex items-start gap-1 text-xs text-destructive">
          <CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />
          <span className="min-w-0 break-words">{status.message}</span>
        </p>
      ) : null}
      {sending ? (
        <p className="text-xs text-muted-foreground">Waiting for the server…</p>
      ) : null}
    </div>
  );
}
