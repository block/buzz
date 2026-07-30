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

  // Plain inline display (not inline-flex): an atomic inline box would be
  // skipped by native text selection (double-click-drag, triple-click), so
  // the link must flow like ordinary text for selection and copy to include
  // it. Long labels wrap with the rest of the message.
  return (
    <a
      className="inline cursor-pointer break-words text-sky-500 hover:underline"
      data-dev-link=""
      href={href}
      onClick={(event) => {
        event.preventDefault();
        void openUrl(href);
      }}
      rel="noreferrer"
      title={href}
    >
      <LinkIcon
        aria-hidden
        className="mr-1 inline-block size-3.5 align-[-0.2em]"
      />
      {label ?? title ?? linkDisplayText(href)}
    </a>
  );
}
