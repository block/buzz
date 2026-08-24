import { useState } from "react";
import { FileKey2, Loader2 } from "lucide-react";

import { invokeTauri } from "@/shared/api/tauri";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

type OwnerAttestationPreview = {
  requestPath: string;
  requestSha256: string;
  schema: string;
  agentPubkey: string;
  agentPublicFingerprintSha256: string;
  ownerPubkey: string;
  conditions: string;
  signingPreimage: string;
  signingHashAlgorithm: string;
  signatureAlgorithm: string;
  resultTagShape: [string, string, string, string];
  resultPath: string;
  validFrom: number;
  expiresAt: number;
  validitySeconds: number;
};

type OwnerAttestationWriteReceipt = {
  requestPath: string;
  requestSha256: string;
  ownerPubkey: string;
  resultPath: string;
  written: boolean;
};

function publicValue(value: string) {
  return (
    <code className="block break-all font-mono text-xs leading-relaxed text-foreground">
      {value}
    </code>
  );
}

function formatUnixTime(value: number) {
  return new Date(value * 1_000).toLocaleString();
}

export function OwnerAttestationSettingsCard() {
  const [preview, setPreview] = useState<OwnerAttestationPreview | null>(null);
  const [receipt, setReceipt] = useState<OwnerAttestationWriteReceipt | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);

  async function chooseRequest() {
    setBusy(true);
    setError(null);
    setReceipt(null);
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
    try {
      const result = await invokeTauri<OwnerAttestationWriteReceipt>(
        "sign_owner_attestation_request",
        {
          requestPath: preview.requestPath,
          expectedRequestSha256: preview.requestSha256,
          expectedOwnerPubkey: preview.ownerPubkey,
        },
      );
      setReceipt(result);
      setPreview(null);
      setConfirmOpen(false);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "The owner attestation was not written.",
      );
      setConfirmOpen(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="min-w-0" data-testid="settings-owner-attestation">
      <SettingsSectionHeader
        title="Owner attestation"
        description="Inspect one existing, bounded NIP-OA request and write its protected auth tag without creating or publishing an agent."
      />

      <SettingsOptionGroupList>
        <SettingsOptionGroup
          title="Request"
          description="Buzz accepts only an owner-controlled 0644 request inside a 0700 custody directory. The authorized result must be its exact BUZZ_AUTH_TAG sibling."
        >
          <SettingsOptionRow>
            <div className="min-w-0 space-y-1">
              <p className="font-medium">OWNER_ATTESTATION_REQUEST.json</p>
              <p className="text-xs text-muted-foreground" data-settings-subcopy>
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
            title="Byte-exact preview"
            description="These public values are re-read and re-bound to the current Desktop owner identity immediately before the protected write."
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
                    Agent fingerprint SHA256 (0x03 + x-only key)
                  </p>
                  {publicValue(preview.agentPublicFingerprintSha256)}
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
                    Signing preimage
                  </p>
                  {publicValue(preview.signingPreimage)}
                </div>
                <div className="grid gap-4 sm:grid-cols-2">
                  <div>
                    <p className="mb-1 text-xs font-medium text-muted-foreground">
                      Valid from
                    </p>
                    <p>{formatUnixTime(preview.validFrom)}</p>
                  </div>
                  <div>
                    <p className="mb-1 text-xs font-medium text-muted-foreground">
                      Expires
                    </p>
                    <p>{formatUnixTime(preview.expiresAt)}</p>
                  </div>
                  <div>
                    <p className="mb-1 text-xs font-medium text-muted-foreground">
                      Validity
                    </p>
                    <p>{preview.validitySeconds.toLocaleString()} seconds</p>
                  </div>
                  <div>
                    <p className="mb-1 text-xs font-medium text-muted-foreground">
                      Algorithms
                    </p>
                    <p>
                      {preview.signingHashAlgorithm} /{" "}
                      {preview.signatureAlgorithm}
                    </p>
                  </div>
                </div>
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">
                    Protected tag shape
                  </p>
                  {publicValue(JSON.stringify(preview.resultTagShape))}
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
              <div className="min-w-0 space-y-1">
                <p className="font-medium">No external effects</p>
                <p className="text-xs text-muted-foreground" data-settings-subcopy>
                  This operation does not create an agent, publish to a relay,
                  use the clipboard, contact Infisical or GitHub, mint a JWT or
                  token, or access VM112. The owner private key and signature
                  stay inside Desktop.
                </p>
              </div>
              <Button disabled={busy} onClick={() => setConfirmOpen(true)}>
                Sign and write protected tag
              </Button>
            </SettingsOptionRow>
          </SettingsOptionGroup>
        ) : null}

        {receipt?.written ? (
          <SettingsOptionGroup title="Completed">
            <SettingsOptionRow>
              <div className="min-w-0 space-y-1">
                <p className="font-medium">Protected tag written once</p>
                <p className="text-xs text-muted-foreground" data-settings-subcopy>
                  The signature and tag value were not returned to the UI.
                </p>
                {publicValue(receipt.resultPath)}
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

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Sign this exact owner attestation?</AlertDialogTitle>
            <AlertDialogDescription>
              Buzz will re-read the request, verify its byte hash, current
              validity, owner identity, file custody, and authorized target,
              then create BUZZ_AUTH_TAG exactly once with mode 0600. An existing
              file or symlink is never replaced.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
            <Button disabled={busy} onClick={() => void signRequest()}>
              {busy ? <Loader2 className="animate-spin" /> : null}
              Sign once
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
