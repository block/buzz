import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

const _agentLiveUpdatesEnabledKey = 'buzz_agent_live_updates_enabled';

/// Whether Buzz should show Live Updates for active agent work.
///
/// This is enabled by default. The operating system's notification and Live
/// Activity settings remain the final authority over whether it surfaces.
class AgentLiveUpdatesEnabledNotifier extends Notifier<bool> {
  @override
  bool build() =>
      ref.read(savedPrefsProvider).getBool(_agentLiveUpdatesEnabledKey) ?? true;

  /// Persists whether agent Live Updates should be shown on this device.
  void setEnabled(bool enabled) {
    state = enabled;
    ref.read(savedPrefsProvider).setBool(_agentLiveUpdatesEnabledKey, enabled);
  }
}

final agentLiveUpdatesEnabledProvider =
    NotifierProvider<AgentLiveUpdatesEnabledNotifier, bool>(
      AgentLiveUpdatesEnabledNotifier.new,
    );
