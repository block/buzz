import { INBOX_COLUMN_MIN_WIDTH_PX } from "@/features/home/useResizableInboxListWidth";

type HomePaneLayoutOptions = {
  activityEnabled: boolean;
  hasAuxiliaryPane: boolean;
  homeWidthPx: number;
  inboxListWidthPx: number;
  isDrafts: boolean;
  isMessagesMode: boolean;
  isNarrow: boolean;
  isReminders: boolean;
  isSinglePanelAuxiliaryView: boolean;
  selectedDraft: boolean;
  selectedEvent: boolean;
  selectedReminder: boolean;
  threadPanelWidthPx: number;
};

export function getHomePaneLayout(options: HomePaneLayoutOptions) {
  const singleMessage =
    options.isMessagesMode &&
    options.isNarrow &&
    options.selectedEvent &&
    !options.isSinglePanelAuxiliaryView;
  const singleDraft =
    options.isDrafts &&
    options.isNarrow &&
    options.selectedDraft &&
    !options.isSinglePanelAuxiliaryView;
  const singleReminder =
    options.activityEnabled &&
    options.isReminders &&
    options.isNarrow &&
    options.selectedReminder &&
    !options.isSinglePanelAuxiliaryView;
  const showList =
    !singleMessage &&
    !singleDraft &&
    !singleReminder &&
    !options.isSinglePanelAuxiliaryView;
  const showDetail =
    !options.isSinglePanelAuxiliaryView &&
    ((options.isMessagesMode && (!options.isNarrow || singleMessage)) ||
      (options.isDrafts && (!options.isNarrow || singleDraft)) ||
      (options.activityEnabled &&
        options.isReminders &&
        (!options.isNarrow || singleReminder)));
  const auxiliaryWidth = options.isSinglePanelAuxiliaryView
    ? options.homeWidthPx
    : options.threadPanelWidthPx;
  const maxListWidth =
    options.homeWidthPx > 0
      ? Math.max(
          INBOX_COLUMN_MIN_WIDTH_PX,
          options.homeWidthPx -
            INBOX_COLUMN_MIN_WIDTH_PX -
            (options.hasAuxiliaryPane ? auxiliaryWidth : 0),
        )
      : undefined;

  return {
    auxiliaryPaneWidthPx: auxiliaryWidth,
    effectiveInboxListWidthPx:
      options.homeWidthPx > 0
        ? Math.min(
            options.inboxListWidthPx,
            maxListWidth ?? options.inboxListWidthPx,
          )
        : options.inboxListWidthPx,
    isSinglePanelDetailView: singleMessage,
    isSinglePanelDraftDetailView: singleDraft,
    isSinglePanelReminderDetailView: singleReminder,
    showDetailPane: showDetail,
    showListPane: showList,
  };
}
