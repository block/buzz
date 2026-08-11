import * as React from "react";

import type { BrowseDialogType } from "@/app/AppShellOverlays";
type CreatedCallback = (channelId: string) => void;

export function useChannelBrowserDialog(onOpen: () => void) {
  const [browseDialogType, setBrowseDialogType] =
    React.useState<BrowseDialogType>(null);
  const createSuccessRef = React.useRef<CreatedCallback | null>(null);
  const [initialTemplateId, setInitialTemplateId] = React.useState<
    string | undefined
  >();
  const openBrowseChannels = React.useCallback(
    (onCreated?: CreatedCallback, nextInitialTemplateId?: string) => {
      createSuccessRef.current = onCreated ?? null;
      setInitialTemplateId(nextInitialTemplateId);
      setBrowseDialogType("stream");
      onOpen();
    },
    [onOpen],
  );

  const onBrowseDialogOpenChange = React.useCallback((open: boolean) => {
    if (!open) {
      createSuccessRef.current = null;
      setInitialTemplateId(undefined);
      setBrowseDialogType(null);
    }
  }, []);

  const getCreateSuccess = React.useCallback(
    () => createSuccessRef.current,
    [],
  );

  return {
    browseDialogType,
    openBrowseChannels,
    onBrowseDialogOpenChange,
    getCreateSuccess,
    initialTemplateId,
  };
}
