import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart' show savedPrefsProvider;

const _showJoinLeaveMessagesKey = 'buzz_show_join_leave_messages';

/// Device-local "Show join and leave messages" preference. Hidden by
/// default: channel timelines omit joined/added/left/removed system rows
/// unless the user enables them in Settings. Purely client-side — the relay
/// always delivers the membership events (member lists depend on them); this
/// setting only controls whether they render in the timeline.
class ShowJoinLeaveMessagesNotifier extends Notifier<bool> {
  @override
  bool build() {
    return ref.read(savedPrefsProvider).getBool(_showJoinLeaveMessagesKey) ??
        false;
  }

  void setEnabled(bool enabled) {
    state = enabled;
    ref.read(savedPrefsProvider).setBool(_showJoinLeaveMessagesKey, enabled);
  }
}

final showJoinLeaveMessagesProvider =
    NotifierProvider<ShowJoinLeaveMessagesNotifier, bool>(
      ShowJoinLeaveMessagesNotifier.new,
    );
