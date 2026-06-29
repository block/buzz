import { BookUser, Download, X } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Separator } from "@/shared/ui/separator";
import { Switch } from "@/shared/ui/switch";

import { PersonaAddedBy } from "./PersonaAddedBy";

type PersonaShareDialogProps = {
  isCatalogVisible: boolean;
  isPending: boolean;
  onCatalogVisibilityChange: (visible: boolean) => void;
  onExport: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  persona: AgentPersona;
};

export function PersonaShareDialog({
  isCatalogVisible,
  isPending,
  onCatalogVisibilityChange,
  onExport,
  onOpenChange,
  open,
  persona,
}: PersonaShareDialogProps) {
  const switchId = `persona-share-catalog-${persona.id}`;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-md"
        data-testid="persona-share-dialog"
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>Catalog options</DialogTitle>
            <div className="flex items-center gap-2">
              <Button
                data-testid="persona-share-export"
                disabled={isPending}
                onClick={onExport}
                size="sm"
                type="button"
                variant="outline"
              >
                <Download className="h-4 w-4" />
                Export
              </Button>
              <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
                <X className="h-4 w-4" />
                <span className="sr-only">Close</span>
              </DialogClose>
            </div>
          </div>
        </DialogHeader>

        <div>
          <div className="flex min-w-0 items-center gap-3 py-4">
            <ProfileAvatar
              avatarUrl={persona.avatarUrl}
              className="h-10 w-10 text-xs"
              label={persona.displayName}
            />
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold leading-snug">
                {persona.displayName}
              </p>
              {persona.isBuiltIn ? null : <PersonaAddedBy className="mt-0.5" />}
            </div>
          </div>

          <Separator />

          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex min-w-0 items-center gap-2">
              <BookUser className="h-4 w-4 shrink-0 text-muted-foreground" />
              <label className="text-sm font-medium" htmlFor={switchId}>
                Show in my catalog
              </label>
            </div>
            <Switch
              checked={isCatalogVisible}
              data-testid="persona-share-show-in-catalog"
              disabled={persona.isBuiltIn || isPending}
              id={switchId}
              onCheckedChange={onCatalogVisibilityChange}
            />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
