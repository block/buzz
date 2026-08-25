import type { BusinessDockConfig } from "@/features/business-dock/businessDockConfig";
import {
  buildBusinessUrl,
  type BusinessResource,
  resolveBusinessResource,
} from "@/features/business-dock/businessResourceResolver";

export type BusinessLinkClickResult = "business" | "external" | "default";

export function handleBuzzLinkClick({
  config,
  event,
  onOpenBusinessResource,
  onOpenExternal,
  url,
}: {
  config: BusinessDockConfig | null;
  event: Pick<MouseEvent, "ctrlKey" | "metaKey" | "preventDefault">;
  onOpenBusinessResource: (resource: BusinessResource) => void;
  onOpenExternal: (url: string) => void;
  url: string;
}): BusinessLinkClickResult {
  if (!config) return "default";
  const resource = resolveBusinessResource(url, config);
  if (!resource) return "default";
  event.preventDefault();
  if (event.metaKey || event.ctrlKey) {
    const externalUrl = buildBusinessUrl(resource, config);
    if (externalUrl) onOpenExternal(externalUrl);
    return "external";
  }
  onOpenBusinessResource(resource);
  return "business";
}
