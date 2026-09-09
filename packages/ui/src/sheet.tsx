import type { ComponentProps } from "react";
import { Dialog } from "./primitives/dialog";
import { skin } from "./classes";
function Popup({ className, ...props }: ComponentProps<typeof Dialog.Popup>) {
  return <Dialog.Popup {...props} className={skin("bui-sheet", className)} />;
}
/** A side sheet uses the same focus, dismissal, title, and description contract as Dialog. */
export const Sheet = { ...Dialog, Popup };
