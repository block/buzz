import * as React from "react";

import { AuxiliaryPanelContext } from "@/shared/layout/auxiliaryPanelContext";
import {
  type AuxiliaryPanelMode,
  getAuxiliaryPanelBodyClass,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";

type AuxiliaryPanelBodyProps = Omit<
  React.ComponentProps<"div">,
  "className"
> & {
  className?: string;
  /** Override mode when rendered outside `AuxiliaryPanel` (e.g. Radix dialog content). */
  mode?: AuxiliaryPanelMode;
  /** Apply top padding in floating overlay (`panel`) mode. */
  panelPadding?: boolean;
};

/** Scroll/content region for auxiliary panels with consistent chrome padding. */
export function AuxiliaryPanelBody({
  className,
  mode: modeOverride,
  panelPadding = false,
  ...props
}: AuxiliaryPanelBodyProps) {
  const context = React.useContext(AuxiliaryPanelContext);
  const mode = modeOverride ?? context?.mode;

  if (mode == null) {
    throw new Error(
      "AuxiliaryPanelBody requires `mode` or an AuxiliaryPanel ancestor",
    );
  }

  return (
    <div
      className={cn(
        "min-h-0 flex-1",
        getAuxiliaryPanelBodyClass({ mode }),
        panelPadding && mode === "panel" && "pt-4",
        className,
      )}
      {...props}
    />
  );
}
