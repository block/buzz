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
  return items.any(
    (item) => !isInboxItemDone(
      item,
      markerOf: readState.effectiveTimestamp,
      localUnreadOverrides: localState.unreadIds,
      localDoneSet: localState.doneIds,
    ),
  );
});
