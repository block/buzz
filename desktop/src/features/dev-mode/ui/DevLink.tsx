import { Link as LinkIcon } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { linkDisplayText } from "@/features/dev-mode/lib/linkDisplay";
import { usePageTitle } from "@/features/dev-mode/lib/usePageTitle";

/**
 * Clickable transcript link: opens in the system browser and renders
 * Slack-style — link icon plus an explicit markdown label when one was
 * written, else the fetched page title, else the cleaned URL.
 */
export function DevLink({ href, label }: { href: string; label?: string }) {
  const title = usePageTitle(label ? null : href);

  return (
    <a
      className="inline-flex max-w-full cursor-pointer items-baseline gap-1 align-baseline text-sky-500 hover:underline"
      href={href}
      onClick={(event) => {
        event.preventDefault();
        void openUrl(href);
      }}
      rel="noreferrer"
      title={href}
    >
      <LinkIcon aria-hidden className="size-3.5 shrink-0 self-center" />
      <span className="truncate">
        {label ?? title ?? linkDisplayText(href)}
      </span>
    </a>
  );
}
