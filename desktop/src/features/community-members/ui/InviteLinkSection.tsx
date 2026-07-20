import { Check, Copy, Link2 } from "lucide-react";
import { QRCodeCanvas } from "qrcode.react";
import * as React from "react";
import { toast } from "sonner";

import { mintInvite } from "@/shared/api/invites";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "@/shared/ui/markdown/MediaContextMenu";

const TTL_OPTIONS: { label: string; value: number }[] = [
  { label: "1 day", value: 24 * 60 * 60 },
  { label: "3 days", value: 3 * 24 * 60 * 60 },
  { label: "7 days", value: 7 * 24 * 60 * 60 },
  { label: "30 days", value: 30 * 24 * 60 * 60 },
];

function formatExpiry(expiresAtUnix: number): string {
  return new Date(expiresAtUnix * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * "Create invite link" section of the relay-access settings card.
 *
 * Mints a stateless invite code via `POST /api/invites` (owner/admin only —
 * the parent card already gates on that) and surfaces the shareable
 * `/invite/<code>` landing-page URL. Codes are multi-use until expiry and
 * are not individually revocable; the relay key is the revocation lever.
 */
export function InviteLinkSection() {
  const [ttlSecs, setTtlSecs] = React.useState(TTL_OPTIONS[1].value);
  const [minting, setMinting] = React.useState(false);
  const [invite, setInvite] = React.useState<{
    url: string;
    expiresAt: number;
  } | null>(null);
  const [copied, setCopied] = React.useState(false);
  const [qrMenu, setQrMenu] = React.useState<MediaContextMenuPosition | null>(
    null,
  );
  const qrRef = React.useRef<HTMLCanvasElement | null>(null);
  const closeQrMenu = React.useCallback(() => setQrMenu(null), []);
  useDismissMediaContextMenu(Boolean(qrMenu), closeQrMenu);

  const ttlLabel =
    TTL_OPTIONS.find((option) => option.value === ttlSecs)?.label ?? "3 days";

  async function handleCreate() {
    setInvite(null);
    setCopied(false);
    setMinting(true);
    try {
      const minted = await mintInvite(ttlSecs);
      setInvite({ url: minted.url, expiresAt: minted.expiresAt });
    } catch (error) {
      toast.error(
        error instanceof Error
          ? `Couldn't create invite link: ${error.message}`
          : "Couldn't create invite link",
      );
    } finally {
      setMinting(false);
    }
  }

  async function handleCopy() {
    if (!invite) return;
    try {
      await writeTextToClipboard(invite.url);
      setCopied(true);
      toast.success("Invite link copied");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Couldn't copy to clipboard");
    }
  }

  async function handleDownloadQr() {
    closeQrMenu();
    const dataUrl = qrRef.current?.toDataURL("image/png");
    if (!dataUrl) return;
    try {
      await invokeTauri("save_png_data_url", {
        dataUrl,
        filename: "buzz-community-invite.png",
      });
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Download failed");
    }
  }

  return (
    <div className="space-y-1.5">
      <span className="text-sm font-medium">Invite link</span>
      <div className="flex flex-col gap-2 sm:flex-row">
        <div className="inline-flex shrink-0 self-stretch sm:self-auto">
          <Button
            className="rounded-r-none flex-1 sm:flex-none"
            data-testid="create-invite-link"
            disabled={minting}
            onClick={() => void handleCreate()}
            type="button"
          >
            <Link2 className="h-4 w-4" />
            Create invite link
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label="Choose invite expiry"
                className="rounded-l-none border-l border-primary-foreground/20 px-2 text-xs"
                data-testid="invite-link-ttl-trigger"
                disabled={minting}
                type="button"
              >
                {ttlLabel}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-40">
              <DropdownMenuLabel>Expires after</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuRadioGroup
                onValueChange={(value) => setTtlSecs(Number(value))}
                value={String(ttlSecs)}
              >
                {TTL_OPTIONS.map((option) => (
                  <DropdownMenuRadioItem
                    data-testid={`invite-link-ttl-${option.value}`}
                    key={option.value}
                    value={String(option.value)}
                  >
                    {option.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        {invite ? (
          <div className="flex min-w-0 flex-1 items-center gap-1 rounded-md border border-border/70 bg-background/70 px-2">
            <span
              className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground"
              data-testid="invite-link-url"
              title={invite.url}
            >
              {invite.url}
            </span>
            <Button
              className="shrink-0"
              data-testid="copy-invite-link"
              onClick={() => void handleCopy()}
              size="icon"
              title="Copy invite link"
              type="button"
              variant="ghost"
            >
              {copied ? (
                <Check className="h-4 w-4 text-green-500" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
              <span className="sr-only">Copy invite link</span>
            </Button>
          </div>
        ) : null}
      </div>
      {invite ? (
        <div className="flex flex-col gap-3 rounded-md border border-border/70 bg-background/70 p-3 sm:flex-row sm:items-center">
          <div className="shrink-0 self-center rounded-md bg-white p-2 text-black">
            <QRCodeCanvas
              ref={qrRef}
              aria-label="Invite QR code"
              data-testid="invite-link-qr-code"
              level="M"
              onContextMenuCapture={(event) => {
                event.preventDefault();
                event.stopPropagation();
                event.nativeEvent.stopImmediatePropagation();
                setQrMenu({ x: event.clientX, y: event.clientY });
              }}
              size={128}
              value={invite.url}
            />
          </div>
          <div className="space-y-1 text-xs text-muted-foreground">
            <p>
              Scan this QR code or share the link above to invite someone to
              this relay.
            </p>
            <p>
              Anyone with this link or QR code can join as a member until{" "}
              {formatExpiry(invite.expiresAt)}.
            </p>
          </div>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          Create a shareable link that lets anyone join this relay as a member
          until it expires.
        </p>
      )}
      {qrMenu ? (
        <MediaContextMenu
          dataAttributes={["data-invite-qr-context-menu"]}
          items={[
            {
              label: "Download image",
              onSelect: () => void handleDownloadQr(),
            },
          ]}
          position={qrMenu}
        />
      ) : null}
    </div>
  );
}
