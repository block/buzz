part of '../channel_detail_page.dart';

/// Agents [currentPubkey] owns that are typing in each thread, by lowercase
/// root message ID, in first-seen order. Like desktop, other typing in a
/// thread stays inside the thread view.
Map<String, List<String>> _threadWorkingAgents(
  WidgetRef ref,
  String? currentPubkey,
  List<TypingEntry> entries,
) {
  final owner = currentPubkey?.toLowerCase();
  if (owner == null || entries.isEmpty) return const {};
  final typers = {for (final entry in entries) entry.pubkey.toLowerCase()};
  // A verified NIP-OA owner on the agent's profile proves ownership.
  final owned = ref
      .watch(
        userCacheProvider.select(
          (cache) => [
            for (final pubkey in typers)
              if (cache[pubkey]?.ownerPubkey?.toLowerCase() == owner) pubkey,
          ].join(','),
        ),
      )
      .split(',');
  final working = <String, List<String>>{};
  for (final entry in entries) {
    final root = entry.threadHeadId?.toLowerCase();
    final pubkey = entry.pubkey.toLowerCase();
    if (root == null || !owned.contains(pubkey)) continue;
    final agents = working.putIfAbsent(root, () => []);
    if (!agents.contains(pubkey)) agents.add(pubkey);
  }
  return working;
}
