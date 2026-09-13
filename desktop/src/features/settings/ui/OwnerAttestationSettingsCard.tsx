import { useState } from "react";
import { FileKey2, Loader2 } from "lucide-react";

import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

type OwnerAttestationPreview = {
  previewId: string;
  agentPubkey: string;
  ownerPubkey: string;
  conditions: string;
  resultPath: string;
};

function publicValue(value: string) {
  return (
    <code className="block break-all font-mono text-xs leading-relaxed text-foreground">
      {value}
    </code>
  );
}

export function OwnerAttestationSettingsCard() {
  const [preview, setPreview] = useState<OwnerAttestationPreview | null>(null);
  const [completedPath, setCompletedPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function chooseRequest() {
    setBusy(true);
    setError(null);
    setCompletedPath(null);
    setPreview(null);
    try {
      const selected = await invokeTauri<OwnerAttestationPreview | null>(
        "select_owner_attestation_request",
      );
      if (selected) setPreview(selected);
    } catch (cause) {
      setPreview(null);
      setError(
        cause instanceof Error
          ? cause.message
          : "The owner attestation request could not be inspected.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function signRequest() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    // The backend consumes this authorization exactly once, including on
    // cancellation or validation failure. Never leave a stale retry surface.
    setPreview(null);
    try {
      await invokeTauri<void>("sign_owner_attestation_request", {
        previewId: preview.previewId,
      });
      setCompletedPath(preview.resultPath);
      setPreview(null);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "The owner attestation was not written.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="min-w-0" data-testid="settings-owner-attestation">
      <SettingsSectionHeader
        title="Owner attestation"
        description="Authorize one existing agent key with the exact conditions in a local request."
      />

      <SettingsOptionGroupList>
        <SettingsOptionGroup
          title="Request"
          description="Buzz accepts an owner-controlled 0644 request inside a 0700 custody directory and writes BUZZ_AUTH_TAG beside it."
        >
          <SettingsOptionRow>
            <div className="min-w-0 space-y-1">
              <p className="font-medium">OWNER_ATTESTATION_REQUEST.json</p>
              <p
                className="text-xs text-muted-foreground"
                data-settings-subcopy
              >
                Selection reads public request data only. It does not sign or
                write anything.
              </p>
            </div>
            <Button
              disabled={busy}
              onClick={() => void chooseRequest()}
              variant="outline"
            >
              {busy && !preview ? (
                <Loader2 className="animate-spin" />
              ) : (
                <FileKey2 />
              )}
              Choose request
            </Button>
          </SettingsOptionRow>
        </SettingsOptionGroup>

        {preview ? (
          <SettingsOptionGroup
            title="Review authorization"
            description="Buzz re-reads these public values immediately before signing."
          >
            <SettingsOptionRow className="items-start">
              <div className="min-w-0 flex-1 space-y-4">
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Agent public key
                  </p>
                  {publicValue(preview.agentPubkey)}
                </div>
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Desktop owner public key
                  </p>
                  {publicValue(preview.ownerPubkey)}
                </div>
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Conditions
                  </p>
                  {publicValue(preview.conditions)}
                </div>
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Authorized result path
                  </p>
                  {publicValue(preview.resultPath)}
                </div>
              </div>
            </SettingsOptionRow>
            <SettingsOptionRow>
              <p
                className="text-xs text-muted-foreground"
                data-settings-subcopy
              >
                The owner private key stays inside Desktop. Nothing is published
                and no agent is created.
              </p>
              <Button disabled={busy} onClick={() => void signRequest()}>
                {busy ? <Loader2 className="animate-spin" /> : null}
                Confirm in Desktop
              </Button>
            </SettingsOptionRow>
          </SettingsOptionGroup>
        ) : null}

        {completedPath ? (
          <SettingsOptionGroup title="Completed">
            <SettingsOptionRow>
              <div className="min-w-0 space-y-1">
                <p className="font-medium">Protected tag written once</p>
                <p
                  className="text-xs text-muted-foreground"
                  data-settings-subcopy
                >
                  The signature and tag value were not returned to the UI.
                </p>
                {publicValue(completedPath)}
              </div>
            </SettingsOptionRow>
          </SettingsOptionGroup>
        ) : null}
      </SettingsOptionGroupList>

      {error ? (
        <p
          aria-live="polite"
          className="mt-6 rounded-lg border border-destructive/35 bg-destructive/10 px-4 py-3 text-sm text-destructive"
          data-testid="owner-attestation-error"
        >
          {error}
        </p>
      ) : null}
    </section>
  );
}
