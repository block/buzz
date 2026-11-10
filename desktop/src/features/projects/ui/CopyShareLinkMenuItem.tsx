import { Link2 } from "lucide-react";

import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";

/**
 * "Copy link" row for the Projects action menus. Renders nothing when the
 * entity has no shareable coordinate (see `lib/projectShareLinks`) so we never
 * offer a link that would fail to parse for the recipient.
 */
export function CopyShareLinkMenuItem({
  label = "Copy link",
  link,
  successMessage = "Link copied to clipboard",
  testId,
}: {
  label?: string;
  link: string | null;
  successMessage?: string;
  testId?: string;
}) {
  if (!link) return null;

  return (
    <DropdownMenuItem
      data-testid={testId}
      onSelect={(event) => {
        event.preventDefault();
        event.stopPropagation();
        copyTextToClipboard(link, successMessage);
      }}
    >
      <Link2 className="h-4 w-4" />
      {label}
    </DropdownMenuItem>
  );
}
