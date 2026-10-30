import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

const _rememberMentionedAgentsKey = 'remember_mentioned_agents';

/// Whether a sent message's notified agents prefill the next draft. Like
/// desktop, it defaults on and is saved on this device.
class RememberMentionedAgentsNotifier extends Notifier<bool> {
  @override
  bool build() =>
      ref.read(savedPrefsProvider).getBool(_rememberMentionedAgentsKey) ?? true;

  void setEnabled(bool enabled) {
    state = enabled;
    ref.read(savedPrefsProvider).setBool(_rememberMentionedAgentsKey, enabled);
  }
}

final rememberMentionedAgentsProvider =
    NotifierProvider<RememberMentionedAgentsNotifier, bool>(
      RememberMentionedAgentsNotifier.new,
    );
