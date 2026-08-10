import * as React from "react";
import { FolderOpen, LoaderCircle, ShieldCheck } from "lucide-react";

import {
  useInstallNxtlinqAuthorizationGatewayMutation,
  useNxtlinqAuthorizationConfigQuery,
  useNxtlinqAuthorizationGatewayQuery,
  useSaveNxtlinqAuthorizationConfigMutation,
} from "@/features/agents/nxtlinqHooks";
import {
  pickNxtlinqDirectory,
  pickNxtlinqTrustStore,
} from "@/shared/api/tauriNxtlinq";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function NxtlinqGatewaySettingsCard() {
  const gatewayQuery = useNxtlinqAuthorizationGatewayQuery();
  const configQuery = useNxtlinqAuthorizationConfigQuery();
  const installMutation = useInstallNxtlinqAuthorizationGatewayMutation();
  const saveMutation = useSaveNxtlinqAuthorizationConfigMutation();
  const [trustStore, setTrustStore] = React.useState("");
  const [receiptRoot, setReceiptRoot] = React.useState("");

  React.useEffect(() => {
    if (!configQuery.data) return;
    setTrustStore(configQuery.data.trustStore ?? "");
    setReceiptRoot(configQuery.data.receiptRoot);
  }, [configQuery.data]);

  async function installGateway(force = false) {
    const result = await installMutation.mutateAsync(force);
    if (result.success) await gatewayQuery.refetch();
  }

  async function chooseTrustStore() {
    const path = await pickNxtlinqTrustStore();
    if (path) setTrustStore(path);
  }

  async function chooseReceiptRoot() {
    const path = await pickNxtlinqDirectory();
    if (path) setReceiptRoot(path);
  }

  async function saveConfig() {
    await saveMutation.mutateAsync({
      trustStore: trustStore.trim() || null,
      receiptRoot: receiptRoot.trim(),
    });
  }

  const installFailure =
    installMutation.data?.success === false
      ? installMutation.data.steps.at(-1)?.hint ||
        installMutation.data.steps.at(-1)?.stderr ||
        "Gateway installation failed."
      : installMutation.error instanceof Error
        ? installMutation.error.message
        : null;

  return (
    <section className="min-w-0" data-testid="settings-nxtlinq-authorization">
      <SettingsSectionHeader
        description="Install the authorization Gateway once and configure operator-owned trust for all local Agents."
        title="Nxtlinq authorization"
      />
      <SettingsOptionGroup>
        <SettingsOptionRow className="items-start">
          <div className="min-w-0 space-y-1">
            <div className="flex items-center gap-2 font-medium">
              <ShieldCheck className="size-4" />
              Gateway
            </div>
            <p className="text-xs text-muted-foreground">
              {gatewayQuery.data?.available
                ? `Installed at ${gatewayQuery.data.resolvedPath ?? "the managed tool path"}`
                : "Not installed"}
            </p>
          </div>
          {!gatewayQuery.data?.available ? (
            <Button
              disabled={installMutation.isPending}
              onClick={() => void installGateway()}
            >
              {installMutation.isPending ? (
                <LoaderCircle className="mr-2 size-4 animate-spin" />
              ) : null}
              Install Gateway
            </Button>
          ) : (
            <Button
              disabled={installMutation.isPending}
              onClick={() => void installGateway(true)}
              variant="outline"
            >
              {installMutation.isPending
                ? "Reinstalling…"
                : "Reinstall Gateway"}
            </Button>
          )}
        </SettingsOptionRow>

        <SettingsOptionRow className="block border-t border-border/50">
          <div className="space-y-2">
            <label
              className="text-sm font-medium"
              htmlFor="nxtlinq-global-trust-store"
            >
              Trusted signers
            </label>
            <div className="flex gap-2">
              <Input
                id="nxtlinq-global-trust-store"
                placeholder="Select trusted-signers.json"
                readOnly
                value={trustStore}
              />
              <Button onClick={() => void chooseTrustStore()} variant="outline">
                <FolderOpen className="mr-2 size-4" />
                Choose file
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              Signer public keys enrolled by this Buzz deployment's operator
              after verifying the project owner's key. Buzz never stores a
              signing private key.
            </p>
          </div>
        </SettingsOptionRow>

        <SettingsOptionRow className="block border-t border-border/50">
          <div className="space-y-2">
            <label
              className="text-sm font-medium"
              htmlFor="nxtlinq-global-receipt-root"
            >
              Receipt storage
            </label>
            <div className="flex gap-2">
              <Input
                id="nxtlinq-global-receipt-root"
                readOnly
                value={receiptRoot}
              />
              <Button
                onClick={() => void chooseReceiptRoot()}
                variant="outline"
              >
                <FolderOpen className="mr-2 size-4" />
                Choose folder
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              Buzz creates a separate owner-only receipt directory for each
              Agent.
            </p>
          </div>
        </SettingsOptionRow>

        <SettingsOptionRow className="justify-end border-t border-border/50">
          <Button
            disabled={
              configQuery.isLoading ||
              saveMutation.isPending ||
              trustStore.trim().length === 0 ||
              receiptRoot.trim().length === 0
            }
            onClick={() => void saveConfig()}
          >
            {saveMutation.isPending ? "Saving…" : "Save Nxtlinq settings"}
          </Button>
        </SettingsOptionRow>
      </SettingsOptionGroup>
      {saveMutation.isSuccess ? (
        <p className="mt-3 text-sm text-emerald-600 dark:text-emerald-400">
          Nxtlinq settings saved for all local Agents.
        </p>
      ) : null}
      {configQuery.error instanceof Error ||
      saveMutation.error instanceof Error ? (
        <p className="mt-3 text-sm text-destructive" role="alert">
          {(saveMutation.error as Error | null)?.message ??
            (configQuery.error as Error | null)?.message}
        </p>
      ) : null}
      {installFailure ? (
        <p className="mt-3 text-sm text-destructive" role="alert">
          {installFailure}
        </p>
      ) : null}
    </section>
  );
}
