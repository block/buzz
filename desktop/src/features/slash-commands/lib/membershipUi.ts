/** `requestUi` name the channel screen handles by opening the member list. */
export const OPEN_MEMBERS_UI = "members";

/** `requestUi` name the sidebar's leave dialog handles. */
export const CONFIRM_LEAVE_CHANNEL_UI = "confirm-leave-channel";

export type ConfirmLeaveChannelRequest = {
  channel: { id: string; name: string };
  /** Called once with the user's choice; later calls are ignored. */
  settle: (confirmed: boolean) => void;
};
