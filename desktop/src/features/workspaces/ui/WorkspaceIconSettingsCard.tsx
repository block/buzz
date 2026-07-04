import { ImagePlus, Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { downscaleIconToDataUrl } from "@/features/workspaces/lib/downscaleIcon";
import {
  useActiveWorkspaceIcon,
  workspaceIconQueryKey,
} from "@/features/workspaces/useWorkspaceIcons";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { workspaceInitials } from "@/features/sidebar/ui/WorkspaceRail";
import { setWorkspaceIcon } from "@/shared/api/workspaceProfile";
import { Button } from "@/shared/ui/button";

const ICON_IMAGE_TYPES = ["image/gif", "image/jpeg", "image/png", "image/webp"];

/**
 * Admin-only workspace icon editor, rendered inside the Relay Access
 * settings section. Publishes a kind:9033 command; the relay stores
 * the icon per community and serves it in NIP-11, which every member's
 * rail reads.
 */
export function WorkspaceIconSettingsCard() {
  const { activeWorkspace } = useWorkspaces();
  const relayUrl = activeWorkspace?.relayUrl;
  const iconQuery = useActiveWorkspaceIcon(relayUrl);
  const queryClient = useQueryClient();
  const inputRef = React.useRef<HTMLInputElement | null>(null);

  const mutation = useMutation({
    mutationFn: (icon: string) => setWorkspaceIcon(icon),
    onSuccess: async (_data, icon) => {
      if (relayUrl) {
        queryClient.setQueryData(workspaceIconQueryKey(relayUrl), icon || null);
        await queryClient.invalidateQueries({
          queryKey: workspaceIconQueryKey(relayUrl),
        });
      }
    },
  });

  async function handleFile(file: File) {
    if (!ICON_IMAGE_TYPES.includes(file.type)) {
      toast.error("Choose a PNG, JPG, GIF, or WebP image.");
      return;
    }
    try {
      const dataUrl = await downscaleIconToDataUrl(file);
      await mutation.mutateAsync(dataUrl);
      toast.success("Workspace icon updated");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update the workspace icon.",
      );
    }
  }

  async function handleClear() {
    try {
      await mutation.mutateAsync("");
      toast.success("Workspace icon removed");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to remove the workspace icon.",
      );
    }
  }

  const icon = iconQuery.data ?? null;
  const initials = activeWorkspace
    ? workspaceInitials(activeWorkspace.name)
    : "";

  return (
    <div className="space-y-1.5" data-testid="workspace-icon-settings">
      <span className="text-sm font-medium">Workspace icon</span>
      <div className="flex items-center gap-3">
        <span className="flex h-12 w-12 shrink-0 items-center justify-center overflow-hidden rounded-xl bg-sidebar-accent/60 text-sm font-semibold text-sidebar-foreground/80">
          {icon ? (
            <img
              alt="Workspace icon"
              className="h-full w-full object-cover"
              data-testid="workspace-icon-preview"
              src={icon}
            />
          ) : (
            initials || "🐝"
          )}
        </span>
        <div className="flex items-center gap-2">
          <Button
            data-testid="workspace-icon-upload"
            disabled={mutation.isPending}
            onClick={() => inputRef.current?.click()}
            type="button"
            variant="outline"
          >
            <ImagePlus className="h-4 w-4" />
            {icon ? "Replace" : "Upload"}
          </Button>
          {icon ? (
            <Button
              data-testid="workspace-icon-remove"
              disabled={mutation.isPending}
              onClick={() => void handleClear()}
              type="button"
              variant="ghost"
            >
              <Trash2 className="h-4 w-4" />
              Remove
            </Button>
          ) : null}
        </div>
        <input
          accept={ICON_IMAGE_TYPES.join(",")}
          className="hidden"
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) void handleFile(file);
          }}
          ref={inputRef}
          type="file"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Shown to every member in the workspace rail and switcher. Square images
        work best.
      </p>
    </div>
  );
}
