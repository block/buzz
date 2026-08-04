import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';
import 'contextual_agent_conversation_policy.dart';

/// Versioned device-local key (matches Desktop).
const unaddressedChannelAgentModeStorageKey =
    'buzz:unaddressed-channel-agent-mode:v1';

UnaddressedChannelAgentMode parseUnaddressedChannelAgentMode(String? raw) {
  switch (raw) {
    case 'mentions-only':
      return UnaddressedChannelAgentMode.mentionsOnly;
    case 'all-channel-agents':
    default:
      return UnaddressedChannelAgentMode.allChannelAgents;
  }
}

String unaddressedChannelAgentModeToStorage(UnaddressedChannelAgentMode mode) {
  switch (mode) {
    case UnaddressedChannelAgentMode.mentionsOnly:
      return 'mentions-only';
    case UnaddressedChannelAgentMode.allChannelAgents:
      return 'all-channel-agents';
  }
}

class UnaddressedChannelAgentModeNotifier
    extends Notifier<UnaddressedChannelAgentMode> {
  @override
  UnaddressedChannelAgentMode build() {
    final prefs = ref.read(savedPrefsProvider);
    return parseUnaddressedChannelAgentMode(
      prefs.getString(unaddressedChannelAgentModeStorageKey),
    );
  }

  void setMode(UnaddressedChannelAgentMode mode) {
    if (state == mode) return;
    state = mode;
    ref
        .read(savedPrefsProvider)
        .setString(
          unaddressedChannelAgentModeStorageKey,
          unaddressedChannelAgentModeToStorage(mode),
        );
  }
}

final unaddressedChannelAgentModeProvider =
    NotifierProvider<
      UnaddressedChannelAgentModeNotifier,
      UnaddressedChannelAgentMode
    >(UnaddressedChannelAgentModeNotifier.new);
