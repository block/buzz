import '../../shared/relay/relay.dart';

/// Whether [event] belongs to the thread rooted at [rootEventId].
bool eventBelongsToThread(NostrEvent event, String rootEventId) {
  if (event.id == rootEventId) {
    return true;
  }
  final ref = event.threadReference;
  return ref.rootId == rootEventId || ref.parentId == rootEventId;
}

/// Resolve the outermost thread root for a reply to [parentEventId].
String resolveReplyRootId(String parentEventId, List<NostrEvent> events) {
  for (final event in events) {
    if (event.id == parentEventId) {
      return event.threadReference.rootId ?? event.id;
    }
  }
  return parentEventId;
}

/// Agent pubkeys that already participate in a thread — authors of thread
/// messages who are agents, and agent pubkeys already p-tagged in that thread.
///
/// Used so a reply without a visible `@` still p-tags those agents and wakes
/// mention-filtered subscriptions (ACP). Does not include humans.
List<String> collectParticipatingAgentPubkeys(
  List<NostrEvent> events,
  String rootEventId,
  bool Function(String pubkey) isAgentPubkey,
) {
  if (rootEventId.isEmpty) {
    return const [];
  }

  final seen = <String>{};
  final result = <String>[];

  void addIfAgent(String? pubkey) {
    if (pubkey == null) {
      return;
    }
    final normalized = pubkey.trim().toLowerCase();
    if (normalized.isEmpty || seen.contains(normalized)) {
      return;
    }
    if (!isAgentPubkey(normalized) && !isAgentPubkey(pubkey)) {
      return;
    }
    seen.add(normalized);
    result.add(normalized);
  }

  for (final event in events) {
    if (!eventBelongsToThread(event, rootEventId)) {
      continue;
    }
    addIfAgent(event.pubkey);
    for (final tag in event.tags) {
      if (tag.isNotEmpty && tag[0] == 'p' && tag.length > 1) {
        addIfAgent(tag[1]);
      }
    }
  }

  return result;
}

/// Merge explicit mention pubkeys with participating thread agents.
List<String> mentionPubkeysWithThreadAgents(
  List<String>? mentionPubkeys,
  List<NostrEvent> events,
  String rootEventId,
  bool Function(String pubkey) isAgentPubkey,
) {
  final participating = collectParticipatingAgentPubkeys(
    events,
    rootEventId,
    isAgentPubkey,
  );
  if (participating.isEmpty) {
    return mentionPubkeys ?? const [];
  }
  return [...?mentionPubkeys, ...participating];
}
