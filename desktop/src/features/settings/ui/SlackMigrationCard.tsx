import { openUrl } from "@tauri-apps/plugin-opener";
import { LogIn } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

const SERVICE_URL_STORAGE_KEY = "buzz.migrate.serviceUrl";

/** Trim, strip trailing slashes, and require a plain http(s) origin. */
function normalizeServiceUrl(raw: string): string | null {
  const trimmed = raw.trim().replace(/\/+$/, "");
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  return trimmed;
}

/**
 * The start side of a Sign-in-with-Slack identity migration. Opening
 * `<service>/oidc/start` needs the person's Buzz pubkey, which only exists on
 * this device — so the flow must begin in the app. On connect we open the
 * operator's claim-service in the system browser with our pubkey; Slack then
 * verifies the user, the service publishes the attestation, and the person is
 * returned via a `buzz://import-claim` deep link (see ImportClaimDialog).
 *
 * Only the public key ever leaves the device; the service URL is whatever the
 * operator shared, remembered locally for next time.
 */
export function SlackMigrationCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const [serviceUrl, setServiceUrl] = useState(() => {
    try {
      return localStorage.getItem(SERVICE_URL_STORAGE_KEY) ?? "";
    } catch {
      return "";
    }
  });
  const [connecting, setConnecting] = useState(false);

  async function handleConnect() {
    const normalized = normalizeServiceUrl(serviceUrl);
    if (!normalized) {
      toast.error("Enter your migration service URL (https://…).");
      return;
    }
    if (!currentPubkey) {
      toast.error("Your identity isn't ready yet — try again in a moment.");
      return;
    }
    try {
      localStorage.setItem(SERVICE_URL_STORAGE_KEY, normalized);
    } catch {
      // Non-fatal: a locked-down storage just means we don't remember the URL.
    }
    setConnecting(true);
    try {
      const startUrl = `${normalized}/oidc/start?pubkey=${encodeURIComponent(
        currentPubkey,
      )}`;
      await openUrl(startUrl);
      toast.success("Continue in your browser to sign in with Slack.");
    } catch (err) {
      toast.error(
        `Couldn't open Slack sign-in: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    } finally {
      setConnecting(false);
    }
  }

  return (
    <section className="min-w-0" data-testid="settings-migrate">
      <SettingsSectionHeader
        title="Migrate from Slack"
        description={
          <>
            Claim the history imported from your Slack workspace so it shows
            under your account. Sign in with Slack to prove it's you — nothing
            is linked until you do, and only your public key leaves this device.
          </>
        }
      />

      <SettingsOptionGroup>
        <SettingsOptionRow className="flex-col items-stretch gap-3">
          <div className="flex items-center gap-3">
            <LogIn className="h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Connect Slack account</p>
              <p className="text-sm font-normal text-muted-foreground">
                Enter the migration service URL your operator shared, then sign
                in with Slack.
              </p>
            </div>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Input
              className="flex-1"
              data-testid="migrate-service-url"
              inputMode="url"
              onChange={(e) => setServiceUrl(e.target.value)}
              placeholder="https://migrate.yourteam.example"
              value={serviceUrl}
            />
            <Button
              data-testid="connect-slack-button"
              disabled={connecting || !currentPubkey || !serviceUrl.trim()}
              onClick={() => void handleConnect()}
              size="sm"
            >
              {connecting ? "Opening…" : "Connect Slack"}
            </Button>
          </div>
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
