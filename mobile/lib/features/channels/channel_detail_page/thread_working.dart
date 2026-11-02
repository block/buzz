part of '../channel_detail_page.dart';

/// Thread typing other than [selfPubkey]'s, as "pubkey root" pairs in
/// first-seen order. Like desktop's joined-key snapshot, comparing this key
/// rather than the entries keeps expiry refreshes from rebuilding the page.
String _threadTypingKey(List<TypingEntry> all, String? selfPubkey) {
  final self = selfPubkey?.toLowerCase();
  return [
    for (final entry in all)
      if (entry.threadHeadId case final root?)
        if (entry.pubkey.toLowerCase() != self)
          '${entry.pubkey.toLowerCase()} ${root.toLowerCase()}',
  ].join(',');
}

/// The user's agents typing in each thread of [typingKey], by lowercase root
/// message ID, in first-seen order. As on desktop, the user's agents are their
/// inventory, archived ones included, read at most
/// [meAgentInventoryMaxAge] ago. Other typing in a thread stays inside the
/// thread view.
Map<String, List<String>> _threadWorkingAgents(
  WidgetRef ref,
  String typingKey,
) {
  if (typingKey.isEmpty) return const {};
  final owned = ref.watch(meAgentInventoryProvider).value;
  if (owned == null) return const {};
  final working = <String, List<String>>{};
  for (final pair in typingKey.split(',')) {
    final [pubkey, root] = pair.split(' ');
    if (!owned.contains(pubkey)) continue;
    final agents = working.putIfAbsent(root, () => []);
    if (!agents.contains(pubkey)) agents.add(pubkey);
  }
  return working;
}
