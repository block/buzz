import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

/// Device-local opt-in, matching the desktop mention picker's default.
const automaticallyMentionAgentsKey = 'buzz.messages.keepMentionedAgentsPinned';

/// Persists the switch before updating the live composer preference.
class AutoMentionAgentsNotifier extends Notifier<bool> {
  @override
  bool build() =>
      ref.read(savedPrefsProvider).getBool(automaticallyMentionAgentsKey) ??
      false;

  /// A failed write leaves the current value intact and is surfaced by the UI.
  Future<void> setEnabled(bool enabled) async {
    final saved = await ref
        .read(savedPrefsProvider)
        .setBool(automaticallyMentionAgentsKey, enabled);
    if (!saved) throw StateError('Could not save the auto-mention setting.');
    if (ref.mounted) state = enabled;
  }
}

final autoMentionAgentsProvider =
    NotifierProvider<AutoMentionAgentsNotifier, bool>(
      AutoMentionAgentsNotifier.new,
    );
