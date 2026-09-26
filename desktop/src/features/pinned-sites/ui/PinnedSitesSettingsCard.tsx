import { BookOpen, MoreHorizontal, Pencil, Plus, Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

import { usePinnedSites } from "../hooks";
import { PINNED_SITE_ICONS, getPinnedSiteIcon } from "../lib/icons";
import type {
  PinnedSite,
  PinnedSiteDraft,
  PinnedSiteIconId,
} from "../lib/types";
import { pinRunbookRef } from "@/features/site-runbook/lib/keys";
import { SiteRunbookDialog } from "@/features/site-runbook/ui/SiteRunbookDialog";

export function PinnedSitesSettingsCard() {
  const { pins, canShareCommunity, isLoading, savePin, deletePin } =
    usePinnedSites();
  const [editing, setEditing] = React.useState<PinnedSite | null>(null);
  const [createOpen, setCreateOpen] = React.useState(false);
  const [deleteTarget, setDeleteTarget] = React.useState<PinnedSite | null>(
    null,
  );

  return (
    <section className="min-w-0" data-testid="settings-pinned-sites">
      <SettingsSectionHeader
        action={
          <Button
            data-testid="pinned-sites-add"
            disabled={isLoading}
            onClick={() => setCreateOpen(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            <Plus className="mr-1.5 h-4 w-4" />
            Add
          </Button>
        }
        description="Put websites in the primary menu. Each pin opens in the main view with no address bar. Community pins are shared with everyone; logins stay on this device."
        title="Pinned sites"
      />

      {isLoading ? (
        <SettingsOptionGroup title="Pins">
          <p className="px-4 py-6 text-center text-sm text-muted-foreground">
            Loading pins...
          </p>
        </SettingsOptionGroup>
      ) : pins.length === 0 ? (
        <SettingsOptionGroup title="Pins">
          <div className="px-4 py-8 text-center text-sm text-muted-foreground">
            No pinned sites yet. Add one to show it in the primary menu.
          </div>
        </SettingsOptionGroup>
      ) : (
        <SettingsOptionGroup title="Pins">
          {pins.map((pin) => (
            <PinRow
              canManage={pin.scope === "personal" || canShareCommunity}
              key={pin.id}
              onDelete={() => setDeleteTarget(pin)}
              onEdit={() => setEditing(pin)}
              pin={pin}
            />
          ))}
        </SettingsOptionGroup>
      )}

      {createOpen ? (
        <PinFormDialog
          canShareCommunity={canShareCommunity}
          onOpenChange={setCreateOpen}
          onSave={async (draft) => {
            await savePin({ draft });
            toast.success("Pinned site added");
          }}
          pin={null}
        />
      ) : null}

      {editing ? (
        <PinFormDialog
          canShareCommunity={canShareCommunity}
          onOpenChange={(open) => {
            if (!open) setEditing(null);
          }}
          onSave={async (draft) => {
            await savePin({ id: editing.id, draft });
            toast.success("Pinned site updated");
          }}
          pin={editing}
        />
      ) : null}

      {deleteTarget ? (
        <AlertDialog
          onOpenChange={(open) => {
            if (!open) setDeleteTarget(null);
          }}
          open
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Delete pinned site</AlertDialogTitle>
              <AlertDialogDescription>
                Remove &quot;{deleteTarget.name}&quot; from the primary menu?
                This does not delete the website.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                onClick={() => {
                  void deletePin(deleteTarget)
                    .then(() => {
                      toast.success(`Deleted "${deleteTarget.name}"`);
                      setDeleteTarget(null);
                    })
                    .catch((error) => {
                      toast.error(
                        error instanceof Error
                          ? error.message
                          : "Failed to delete pin",
                      );
                    });
                }}
              >
                Delete
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </section>
  );
}

function PinRow({
  canManage,
  onDelete,
  onEdit,
  pin,
}: {
  canManage: boolean;
  onDelete: () => void;
  onEdit: () => void;
  pin: PinnedSite;
}) {
  const Icon = getPinnedSiteIcon(pin.icon);
  const [runbookOpen, setRunbookOpen] = React.useState(false);

  return (
    <div
      className="group flex items-center gap-3 px-4 py-3"
      data-testid={`pinned-site-row-${pin.id}`}
    >
      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{pin.name}</span>
          {pin.scope === "community" ? (
            <Badge className="shrink-0 text-2xs" variant="outline">
              Community
            </Badge>
          ) : null}
        </div>
        <p
          className="truncate text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          {pin.url}
        </p>
      </div>
      {canManage ? (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={`Actions for ${pin.name}`}
              className="h-7 w-7 shrink-0"
              size="icon"
              type="button"
              variant="ghost"
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={onEdit}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuItem
              data-testid={`pinned-site-runbook-${pin.id}`}
              onClick={() => setRunbookOpen(true)}
            >
              <BookOpen className="mr-2 h-4 w-4" />
              How to use this site
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onDelete}>
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
      <SiteRunbookDialog
        onOpenChange={setRunbookOpen}
        open={runbookOpen}
        runbookRef={pinRunbookRef(pin.id)}
        title={`How to use ${pin.name}`}
      />
    </div>
  );
}

function PinFormDialog({
  canShareCommunity,
  onOpenChange,
  onSave,
  pin,
}: {
  canShareCommunity: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (draft: PinnedSiteDraft) => Promise<void>;
  pin: PinnedSite | null;
}) {
  const [name, setName] = React.useState(pin?.name ?? "");
  const [url, setUrl] = React.useState(pin?.url ?? "");
  const [icon, setIcon] = React.useState<PinnedSiteIconId>(
    pin?.icon ?? "compass",
  );
  const [pollForChanges, setPollForChanges] = React.useState(
    pin?.pollForChanges ?? false,
  );
  const [openMatchingLinks, setOpenMatchingLinks] = React.useState(
    pin?.openMatchingLinks !== false,
  );
  const [community, setCommunity] = React.useState(pin?.scope === "community");
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await onSave({
        name,
        url,
        icon,
        pollForChanges,
        openMatchingLinks,
        community,
      });
      onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Failed to save pin");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open>
      <ChooserDialogContent
        footer={
          <div className="flex justify-end gap-2">
            <Button
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="pinned-site-save"
              disabled={saving}
              form="pinned-site-form"
              type="submit"
            >
              {pin ? "Save" : "Add"}
            </Button>
          </div>
        }
        title={pin ? "Edit pinned site" : "Add pinned site"}
      >
        <form
          className="space-y-4"
          id="pinned-site-form"
          onSubmit={(event) => void handleSubmit(event)}
        >
          <div className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor="pinned-site-name">
              Name
            </label>
            <Input
              data-testid="pinned-site-name"
              id="pinned-site-name"
              onChange={(event) => setName(event.target.value)}
              placeholder="Wayfinder"
              value={name}
            />
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor="pinned-site-url">
              URL
            </label>
            <Input
              data-testid="pinned-site-url"
              id="pinned-site-url"
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://example.com"
              value={url}
            />
          </div>
          <div className="space-y-1.5">
            <p className="text-sm font-medium">Icon</p>
            <div
              className="grid grid-cols-8 gap-1.5"
              data-testid="pinned-site-icon-picker"
            >
              {PINNED_SITE_ICONS.map((entry) => {
                const Icon = getPinnedSiteIcon(entry.id);
                const selected = icon === entry.id;
                return (
                  <button
                    aria-label={entry.label}
                    aria-pressed={selected}
                    className={
                      selected
                        ? "flex h-8 w-8 items-center justify-center rounded-md bg-primary/15 text-primary"
                        : "flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted"
                    }
                    data-testid={`pinned-site-icon-${entry.id}`}
                    key={entry.id}
                    onClick={() => setIcon(entry.id)}
                    type="button"
                  >
                    <Icon className="h-4 w-4" />
                  </button>
                );
              })}
            </div>
          </div>
          <SettingsOptionRow className="rounded-lg border border-border/70 px-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Poll for changes</p>
              <p className="text-xs text-muted-foreground/70">
                Recheck the start URL about once a minute and refresh if it
                changed.
              </p>
            </div>
            <Switch
              checked={pollForChanges}
              data-testid="pinned-site-poll"
              onCheckedChange={setPollForChanges}
            />
          </SettingsOptionRow>
          <SettingsOptionRow className="rounded-lg border border-border/70 px-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Open matching links here</p>
              <p className="text-xs text-muted-foreground/70">
                When on, left-click links for this pin&apos;s domain open in this
                pinned site, and right-click offers &quot;Open in Pinned
                Website&quot;. When off, this pin is skipped for domain routing
                (other pins may still match).
              </p>
            </div>
            <Switch
              checked={openMatchingLinks}
              data-testid="pinned-site-open-matching-links"
              onCheckedChange={setOpenMatchingLinks}
            />
          </SettingsOptionRow>
          <SettingsOptionRow className="rounded-lg border border-border/70 px-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Share with community</p>
              <p className="text-xs text-muted-foreground/70">
                {canShareCommunity
                  ? "Everyone in this community sees the name, URL, and icon. Logins stay on each person's device."
                  : "Only a community owner or admin can share a pin."}
              </p>
            </div>
            <Switch
              checked={community}
              data-testid="pinned-site-community"
              disabled={!canShareCommunity}
              onCheckedChange={setCommunity}
            />
          </SettingsOptionRow>
          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
