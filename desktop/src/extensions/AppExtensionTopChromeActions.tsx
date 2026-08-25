import { BusinessDockTopChromeAction } from "@/features/business-dock/BusinessDockTopChromeAction";

/** Product-specific chrome actions kept behind one stable Buzz integration point. */
export function AppExtensionTopChromeActions() {
  return <BusinessDockTopChromeAction />;
}
