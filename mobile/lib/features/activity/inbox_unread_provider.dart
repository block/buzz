import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/read_state/read_state_provider.dart';
import 'activity_provider.dart';
import 'inbox_item.dart';
import 'inbox_local_state_provider.dart';
import 'inbox_read_state.dart';

/// Whether Activity has unread rows, or read state has not loaded yet.
enum InboxUnread { pending, read, unread }

/// The shared unread rule for the Activity bell and app icon badge.
final inboxUnreadProvider = Provider<InboxUnread>((ref) {
  final items = ref.watch(inboxItemsProvider);
  final readState = ref.watch(readStateProvider);
  final localState = ref.watch(inboxLocalStateProvider);
  if (!readState.isReady) return InboxUnread.pending;
  // A mark any device has read past is stale: remove it, not just ignore it.
  final stale = [
    for (final item in items)
      for (final id in groupedInboxItemIds(item))
        if (localState.unreadBaselines[id] case final baseline?
            when inboxItemReadAt(
                  item,
                  markerOf: (contextId) => readState.contexts[contextId],
                ) >
                baseline)
          id,
  ];
  if (stale.isNotEmpty) {
    Future.microtask(
      () => ref.read(inboxLocalStateProvider.notifier).clearUnread(stale),
    );
  }
  return _inboxUnread(items, readState, localState);
});

/// [inboxUnreadProvider]'s rule over only the events the channel list
/// received, which always belong to the current community. The icon uses it
/// while the Activity feed loads.
final observedInboxUnreadProvider = Provider<InboxUnread>((ref) {
  final items = ref.watch(observedInboxItemsProvider);
  final readState = ref.watch(readStateProvider);
  final localState = ref.watch(inboxLocalStateProvider);
  if (!readState.isReady) return InboxUnread.pending;
  return _inboxUnread(items, readState, localState);
});

InboxUnread _inboxUnread(
  List<InboxItem> items,
  ReadStateState readState,
  InboxLocalState localState,
) {
  final anyUnread = items.any(
    (item) => !isInboxItemDone(
      item,
      markerOf: readState.effectiveTimestamp,
      rawMarkerOf: (contextId) => readState.contexts[contextId],
      localUnreadOverrides: localState.unreadBaselines,
      localDoneSet: localState.doneIds,
    ),
  );
  return anyUnread ? InboxUnread.unread : InboxUnread.read;
}

/// The bell reflects unread Activity rows; pending read state shows no dot.
final inboxHasUnreadProvider = Provider<bool>(
  (ref) => ref.watch(inboxUnreadProvider) == InboxUnread.unread,
);
