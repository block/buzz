import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_assistants.dart';
import 'channel_typing_provider.dart';

/// The conversation a composer belongs to: a channel, or one of its threads.
class ActivityScope {
  const ActivityScope({
    required this.channelId,
    required this.selfPubkey,
    this.threadRootId,
    this.allThreads = false,
  });

  final String channelId;
  final String? selfPubkey;

  /// Null for the channel conversation.
  final String? threadRootId;

  /// Counts typing in any thread as this conversation's, as Me does.
  final bool allThreads;
}

/// Who is active in a conversation, as desktop's composer shows it.
typedef ConversationActivityState = ({
  /// The user's agents typing here, sorted by pubkey as on desktop, each with
  /// the thread roots it types in. A null root is the channel conversation.
  Map<String, List<String?>> agents,

  /// Everyone else typing in this conversation itself, in first-seen order.
  List<String> typers,
});

const _idle = (agents: <String, List<String?>>{}, typers: <String>[]);

/// Desktop's composer activity for [scope], from live typing only. As on
/// desktop, the user's agents are their inventory, and a channel view counts
/// those agents' thread work while a thread view does not. Null while the
/// inventory first loads, so an agent does not show as a typing person
/// before it is known.
ConversationActivityState? watchConversationActivity(
  WidgetRef ref,
  ActivityScope scope,
) {
  final threadRootId = scope.threadRootId;
  final entries = ref
      .watch(
        visibleTypingProvider(
          scope.channelId,
          selfPubkey: scope.selfPubkey,
          include: (entry) =>
              threadRootId == null || entry.threadHeadId == threadRootId,
        ),
      )
      .entries;
  if (entries.isEmpty) return _idle;
  final inventory = ref.watch(meAgentInventoryProvider);
  if (inventory.isLoading && !inventory.hasValue) return null;
  final owned = {
    for (final pubkey in inventory.value ?? const <String>{})
      pubkey.toLowerCase(),
  };
  final agents = <String, List<String?>>{};
  final typers = <String>[];
  for (final entry in entries) {
    final pubkey = entry.pubkey.toLowerCase();
    if (owned.contains(pubkey)) {
      final roots = agents.putIfAbsent(pubkey, () => []);
      if (!roots.contains(entry.threadHeadId)) roots.add(entry.threadHeadId);
    } else if ((scope.allThreads || entry.threadHeadId == threadRootId) &&
        !typers.contains(pubkey)) {
      typers.add(pubkey);
    }
  }
  final sorted = agents.keys.toList()..sort();
  return (
    agents: {for (final pubkey in sorted) pubkey: agents[pubkey]!},
    typers: [
      for (final pubkey in typers)
        if (!agents.containsKey(pubkey)) pubkey,
    ],
  );
}
