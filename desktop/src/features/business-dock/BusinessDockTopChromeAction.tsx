import * as React from "react";
import { BriefcaseBusiness } from "lucide-react";

import { useBusinessDock } from "@/features/business-dock/BusinessDockProvider";
import { isBusinessDockShortcut } from "@/features/business-dock/businessDockShortcut";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

const TOP_CHROME_ACTION_CLASS =
  "h-[28px] w-[28px] rounded-[4px] text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground";

export function BusinessDockTopChromeAction() {
  const { state, toggle } = useBusinessDock();

  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isBusinessDockShortcut(event)) return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggle]);

  return (
    <Button
      aria-label="Toggle Business Dock"
      aria-pressed={state.open}
      className={cn(
        TOP_CHROME_ACTION_CLASS,
        state.open && "bg-sidebar-accent text-sidebar-accent-foreground",
      )}
      data-testid="business-dock-toggle"
      onClick={toggle}
      size="icon"
      title="Business system (⌘⇧B / Ctrl+Shift+B)"
      type="button"
      variant="ghost"
    >
      <BriefcaseBusiness />
      <span className="sr-only">Toggle Business Dock</span>
    </Button>
  );
}
