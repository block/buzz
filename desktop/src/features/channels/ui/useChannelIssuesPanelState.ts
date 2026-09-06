import * as React from "react";

export function useChannelIssuesPanelState() {
  const [open, setOpen] = React.useState(false);
  const close = React.useCallback(() => setOpen(false), []);
  const toggle = React.useCallback(() => setOpen((current) => !current), []);

  return { close, open, setOpen, toggle };
}

export function useChannelIssuesPanelToggle({
  closeAgentSession,
  setChannelManagementOpen,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  toggleIssues,
}: {
  closeAgentSession: () => void;
  setChannelManagementOpen: (open: boolean) => void;
  setOpenThreadHeadId: (eventId: string | null) => void;
  setProfilePanelPubkey: (pubkey: string | null) => void;
  toggleIssues: () => void;
}) {
  return React.useCallback(
    (options?: { preserveThread?: boolean }) => {
      toggleIssues();
      if (!options?.preserveThread) setOpenThreadHeadId(null);
      setProfilePanelPubkey(null);
      setChannelManagementOpen(false);
      closeAgentSession();
    },
    [
      closeAgentSession,
      setChannelManagementOpen,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      toggleIssues,
    ],
  );
}
