import * as React from "react";

import {
  gifPickerSlashUiName,
  readSettledUiRequest,
} from "@/features/slash-commands/lib/slashUiRequest";
import { useSlashCommandUiHandler } from "@/features/slash-commands/lib/slashCommandUiRequests";

const NO_SLASH_COMMANDS_UI = "gif-picker:none";

/**
 * Lets `/gif` open this composer's picker on the GIFs tab. `gifQuery` is the
 * search to pre-fill, or null while the picker was opened by hand.
 */
export function useComposerGifRequest({
  blockedReason,
  channelId,
  enabled,
  isPickerOpen,
  isThread,
  openPicker,
}: {
  blockedReason: string | null;
  channelId: string | null;
  enabled: boolean;
  isPickerOpen: boolean;
  isThread: boolean;
  openPicker: () => void;
}) {
  const [gifQuery, setGifQuery] = React.useState<string | null>(null);

  useSlashCommandUiHandler(
    enabled && channelId
      ? gifPickerSlashUiName(channelId, isThread)
      : NO_SLASH_COMMANDS_UI,
    (payload) => {
      const request = readSettledUiRequest<{ query: string }>(payload);
      if (!request) return;
      if (blockedReason) {
        request.settle(new Error(blockedReason));
        return;
      }
      setGifQuery(request.query);
      openPicker();
      request.settle();
    },
  );

  React.useEffect(() => {
    if (!isPickerOpen) setGifQuery(null);
  }, [isPickerOpen]);

  return gifQuery;
}
