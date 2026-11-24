import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/me/me_assistant_names.dart';
import '../../../shared/me/me_assistants.dart';
import '../channel_typing_provider.dart';

/// Only verified owned agents contribute a Solo working signal. The existing
/// typing provider clears replies, expired signals, and disconnected sessions.
/// Thread activity belongs to the containing Solo chat too.
final meWorkingAgentsProvider = Provider.autoDispose
    .family<List<String>, String>((ref, channelId) {
      final agents = ref.watch(meAssistantsProvider).value ?? [];
      if (agents.isEmpty) return const [];
      final typing = ref.watch(channelTypingProvider(channelId));
      final active = typing.map((entry) => entry.pubkey.toLowerCase()).toSet();
      final working = [
        for (final agent in agents)
          if (active.contains(agent.pubkey.toLowerCase())) agent.pubkey,
      ];
      // An idle chat reads no names at all.
      if (working.isEmpty) return const [];
      final names = ref.watch(meAssistantNamesProvider);
      return [for (final pubkey in working) names.labelFor(pubkey)];
    });
