import type * as React from "react";

import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

type AgentDefinitionDialogShellProps = {
  children: React.ReactNode;
  description: string;
  embedded: boolean;
  footer: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  title: string;
};

export function AgentDefinitionDialogShell({
  children,
  description,
  embedded,
  footer,
  onOpenChange,
  open,
  title,
}: AgentDefinitionDialogShellProps) {
  if (embedded) {
    return (
      <div
        className="relative flex min-h-0 min-w-0 flex-1 flex-col"
        data-testid="persona-dialog"
      >
        <div className="min-h-0 min-w-0 flex-1 overflow-y-auto px-5 pb-20 pt-5">
          {children}
        </div>
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-end bg-linear-to-t from-background via-background/95 to-transparent px-4 pb-3 pt-10 [&_button]:pointer-events-auto">
          {footer}
        </div>
      </div>
    );
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-3xl border-0"
        contentClassName="pt-3"
        data-testid="persona-dialog"
        description={description}
        footer={footer}
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title={title}
      >
        {children}
      </ChooserDialogContent>
    </Dialog>
  );
}
