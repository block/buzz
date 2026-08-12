import * as React from "react";
import { Archive, Plus, ShieldCheck } from "lucide-react";

import {
  useArchiveExternalAgentRuntimeMutation,
  useExternalAgentRuntimesQuery,
  useRegisterExternalAgentRuntimeMutation,
} from "@/features/agents/hooks";
import type { ExternalAgentRuntime } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/shared/ui/card";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

type FormState = {
  agentPubkey: string;
  ownerAuthTag: string;
  name: string;
  purpose: string;
  deploymentScope: string;
  runnerOwner: string;
  healthSource: string;
  shutdownPath: string;
  allowedChannels: string;
  rateLimitPerMinute: string;
  retirementDate: string;
};

const EMPTY_FORM: FormState = {
  agentPubkey: "",
  ownerAuthTag: "",
  name: "",
  purpose: "",
  deploymentScope: "",
  runnerOwner: "",
  healthSource: "",
  shutdownPath: "",
  allowedChannels: "",
  rateLimitPerMinute: "12",
  retirementDate: "",
};

export function ExternalRuntimesSection() {
  const runtimesQuery = useExternalAgentRuntimesQuery();
  const registerMutation = useRegisterExternalAgentRuntimeMutation();
  const archiveMutation = useArchiveExternalAgentRuntimeMutation();
  const [open, setOpen] = React.useState(false);
  const [form, setForm] = React.useState<FormState>(EMPTY_FORM);
  const [formError, setFormError] = React.useState<string | null>(null);

  function update(field: keyof FormState, value: string) {
    setForm((current) => ({ ...current, [field]: value }));
  }

  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFormError(null);
    const allowedChannels = form.allowedChannels
      .split(/[\n,]/u)
      .map((channel) => channel.trim())
      .filter(Boolean);
    const rateLimit = Number.parseInt(form.rateLimitPerMinute, 10);
    if (!Number.isInteger(rateLimit) || rateLimit < 1 || rateLimit > 60) {
      setFormError("Rate limit must be between 1 and 60 messages per minute.");
      return;
    }
    if (allowedChannels.length === 0) {
      setFormError("Add at least one allowed channel.");
      return;
    }
    try {
      await registerMutation.mutateAsync({
        agentPubkey: form.agentPubkey,
        ownerAuthTag: form.ownerAuthTag,
        name: form.name,
        purpose: form.purpose,
        deploymentScope: form.deploymentScope,
        runnerOwner: form.runnerOwner,
        healthSource: form.healthSource,
        shutdownPath: form.shutdownPath,
        allowedChannels,
        mentionOnly: true,
        mentionFilter: true,
        rateLimitPerMinute: rateLimit,
        retirementDate: form.retirementDate,
      });
      setForm(EMPTY_FORM);
      setOpen(false);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : String(error));
    }
  }

  const runtimes = runtimesQuery.data ?? [];

  return (
    <>
      <Card data-testid="external-runtimes-section">
        <CardHeader className="flex-row items-start justify-between gap-4">
          <div className="space-y-1.5">
            <CardTitle className="text-lg">External runtimes</CardTitle>
            <CardDescription>
              Provenance and shutdown register only. Registration never imports
              a key or starts a Buzz executor.
            </CardDescription>
          </div>
          <Button
            data-testid="external-runtime-register-button"
            onClick={() => {
              setFormError(null);
              setOpen(true);
            }}
            size="sm"
          >
            <Plus />
            Register
          </Button>
        </CardHeader>
        <CardContent className="space-y-3">
          {runtimesQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">Loading register…</p>
          ) : runtimesQuery.error instanceof Error ? (
            <p className="text-sm text-destructive">
              {runtimesQuery.error.message}
            </p>
          ) : runtimes.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No external runner is registered on this device.
            </p>
          ) : (
            runtimes.map((runtime) => (
              <ExternalRuntimeRow
                archivePending={archiveMutation.isPending}
                key={[runtime.agentPubkey, runtime.deploymentScope].join(":")}
                runtime={runtime}
                onArchive={() => {
                  void archiveMutation.mutateAsync(runtime.agentPubkey);
                }}
              />
            ))
          )}
        </CardContent>
      </Card>

      <Dialog
        open={open}
        onOpenChange={(nextOpen) => {
          if (!registerMutation.isPending) setOpen(nextOpen);
        }}
      >
        <DialogContent
          className="max-h-[calc(100vh-2rem)] max-w-2xl overflow-y-auto"
          data-testid="external-runtime-register-dialog"
        >
          <DialogHeader>
            <DialogTitle>Register an external runner</DialogTitle>
            <DialogDescription>
              Paste the owner-signed NIP-OA tag for this public identity. Buzz
              stores only the verified owner and operating contract; it never
              receives the runner key.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={submit}>
            <div className="grid gap-3 sm:grid-cols-2">
              <Field
                id="external-runtime-agent-pubkey"
                label="Agent npub or hex"
                value={form.agentPubkey}
                onChange={(value) => update("agentPubkey", value)}
              />
              <Field
                id="external-runtime-deployment-scope"
                label="Deployment scope"
                value={form.deploymentScope}
                onChange={(value) => update("deploymentScope", value)}
              />
              <Field
                id="external-runtime-name"
                label="Name"
                value={form.name}
                onChange={(value) => update("name", value)}
              />
              <Field
                id="external-runtime-runner-owner"
                label="Runner owner"
                value={form.runnerOwner}
                onChange={(value) => update("runnerOwner", value)}
              />
              <Field
                id="external-runtime-health-source"
                label="Health source"
                value={form.healthSource}
                onChange={(value) => update("healthSource", value)}
              />
              <Field
                id="external-runtime-shutdown-path"
                label="Shutdown path"
                value={form.shutdownPath}
                onChange={(value) => update("shutdownPath", value)}
              />
              <Field
                id="external-runtime-rate-limit"
                label="Rate limit / minute"
                type="number"
                value={form.rateLimitPerMinute}
                onChange={(value) => update("rateLimitPerMinute", value)}
              />
              <Field
                id="external-runtime-retirement-date"
                label="Retirement date"
                value={form.retirementDate}
                onChange={(value) => update("retirementDate", value)}
              />
            </div>
            <TextField
              id="external-runtime-purpose"
              label="Purpose"
              value={form.purpose}
              onChange={(value) => update("purpose", value)}
            />
            <TextField
              id="external-runtime-allowed-channels"
              label="Allowed channels (comma or newline separated)"
              value={form.allowedChannels}
              onChange={(value) => update("allowedChannels", value)}
            />
            <label
              className="block space-y-1.5 text-sm"
              htmlFor="external-runtime-owner-auth-tag"
            >
              <span className="font-medium">Owner auth tag</span>
              <Textarea
                autoComplete="off"
                id="external-runtime-owner-auth-tag"
                required
                value={form.ownerAuthTag}
                onChange={(event) => update("ownerAuthTag", event.target.value)}
              />
              <span className="text-xs text-muted-foreground">
                Verified once, then discarded. It is not written to the local
                register or published in the provenance projection.
              </span>
            </label>
            {formError ? (
              <p className="text-sm text-destructive">{formError}</p>
            ) : null}
            <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
              <ShieldCheck className="h-4 w-4 shrink-0" />
              <span>
                The runner remains owned by its current launcher. This action
                does not grant business authority, membership, compute, or
                external-send permission.
              </span>
            </div>
            <DialogFooter>
              <DialogClose asChild>
                <Button
                  disabled={registerMutation.isPending}
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
              </DialogClose>
              <Button disabled={registerMutation.isPending} type="submit">
                {registerMutation.isPending
                  ? "Verifying…"
                  : "Verify and register"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}

function ExternalRuntimeRow({
  runtime,
  archivePending,
  onArchive,
}: {
  runtime: ExternalAgentRuntime;
  archivePending: boolean;
  onArchive: () => void;
}) {
  return (
    <div className="rounded-lg border border-border/70 px-3 py-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className="font-medium">{runtime.name}</p>
            <Badge variant={runtime.archived ? "secondary" : "success"}>
              {runtime.archived ? "Archived" : "Active register"}
            </Badge>
          </div>
          <p className="break-all font-mono text-xs text-muted-foreground">
            {runtime.agentPubkey}
          </p>
          <p className="break-all font-mono text-xs text-muted-foreground">
            owner {runtime.ownerPubkey}
          </p>
          <p className="text-sm text-muted-foreground">
            {runtime.deploymentScope}
          </p>
        </div>
        {!runtime.archived ? (
          <Button
            disabled={archivePending}
            onClick={onArchive}
            size="sm"
            type="button"
            variant="outline"
          >
            <Archive />
            Archive register
          </Button>
        ) : null}
      </div>
      <div className="mt-2 grid gap-1 text-xs text-muted-foreground sm:grid-cols-2">
        <span className="sm:col-span-2">Purpose: {runtime.purpose}</span>
        <span>Write allowlist: {runtime.allowedChannels.join(", ")}</span>
        <span>Runner owner: {runtime.runnerOwner}</span>
        <span>Health: {runtime.healthSource}</span>
        <span>Shutdown: {runtime.shutdownPath}</span>
        <span>Retirement: {runtime.retirementDate}</span>
      </div>
    </div>
  );
}

function Field({
  id,
  label,
  value,
  onChange,
  type = "text",
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: React.HTMLInputTypeAttribute;
}) {
  return (
    <label className="block space-y-1.5 text-sm" htmlFor={id}>
      <span className="font-medium">{label}</span>
      <Input
        id={id}
        required
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function TextField({
  id,
  label,
  value,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="block space-y-1.5 text-sm" htmlFor={id}>
      <span className="font-medium">{label}</span>
      <Textarea
        id={id}
        required
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}
