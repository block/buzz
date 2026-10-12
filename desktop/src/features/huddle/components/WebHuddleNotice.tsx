import type * as React from "react";

import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

export const WEB_HUDDLE_NOTICE =
  "Huddles aren't available in the web app yet. Join from the Buzz desktop app.";

/** Web: tapping a read-only huddle or wave card explains that huddles live on the desktop. */
export function WebHuddleNotice({ children }: { children: React.ReactNode }) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <div
          className="cursor-pointer"
          data-testid="web-huddle-notice-trigger"
          role="button"
          tabIndex={0}
        >
          {children}
        </div>
      </PopoverTrigger>
      <PopoverContent
        className="w-72 text-sm"
        data-testid="web-huddle-notice"
        side="top"
      >
        {WEB_HUDDLE_NOTICE}
      </PopoverContent>
    </Popover>
  );
}
