import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../channel_management_provider.dart';
import '../channel_typing_provider.dart';
import 'active_agent_turns.dart';

typedef _WorkingBotPubkeys = ({Set<String> anywhere, Set<String> channelRoot});

final _workingBotPubkeysByScopeProvider =
    Provider.family<_WorkingBotPubkeys, String>((ref, channelId) {
      final members = ref.watch(channelMembersProvider(channelId));
      final activeTurns = ref.watch(activeAgentTurnsProvider);
      final typingEntries = ref.watch(channelTypingProvider(channelId));
      final botPubkeys = {
        for (final member in members.asData?.value ?? const <ChannelMember>[])
          if (member.isBot) member.pubkey.toLowerCase(),
      };
      final observerPubkeys = {
        for (final turn in activeTurns)
          if (turn.channelId == channelId &&
              botPubkeys.contains(turn.agentPubkey.toLowerCase()))
            turn.agentPubkey.toLowerCase(),
      };

      return (
        anywhere: {
          ...observerPubkeys,
          for (final entry in typingEntries)
            if (botPubkeys.contains(entry.pubkey.toLowerCase()))
              entry.pubkey.toLowerCase(),
        },
        channelRoot: {
          ...observerPubkeys,
          for (final entry in typingEntries)
            if (entry.threadHeadId == null &&
                botPubkeys.contains(entry.pubkey.toLowerCase()))
              entry.pubkey.toLowerCase(),
        },
      );
    });

/// Bot members working anywhere in a channel, including its threads.
///
/// Observer-backed turns provide lifecycle state. Typing remains a compatibility
/// fallback for harnesses that do not publish observer frames.
/// Used by the members badge and members sheet.
final workingBotPubkeysProvider = Provider.family<Set<String>, String>((
  ref,
  channelId,
) {
  return ref.watch(_workingBotPubkeysByScopeProvider(channelId)).anywhere;
});

/// Bot members whose work belongs in the main channel activity row.
///
/// Thread-scoped typing stays in the thread while observer turns remain visible
/// for their channel lifecycle.
final channelWorkingBotPubkeysProvider = Provider.family<Set<String>, String>((
  ref,
  channelId,
) {
  return ref.watch(_workingBotPubkeysByScopeProvider(channelId)).channelRoot;
});
