import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/read_state/read_state_provider.dart';
import 'activity_provider.dart';
import 'inbox_local_state_provider.dart';
import 'inbox_read_state.dart';

/// The bell reflects the same unread rows shown by the Activity inbox.
final inboxHasUnreadProvider = Provider<bool>((ref) {
  final items = ref.watch(inboxItemsProvider);
  final readState = ref.watch(readStateProvider);
  final localState = ref.watch(inboxLocalStateProvider);
  if (!readState.isReady) return false;
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
  return items.any(
    (item) => !isInboxItemDone(
      item,
      markerOf: readState.effectiveTimestamp,
      rawMarkerOf: (contextId) => readState.contexts[contextId],
      localUnreadOverrides: localState.unreadBaselines,
      localDoneSet: localState.doneIds,
    ),
  );
});
