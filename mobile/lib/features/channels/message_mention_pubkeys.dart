/// Return the semantic recipients for an outgoing message.
///
/// Stream messages notify only explicit mentions. A DM addresses every other
/// participant, so it must carry recipient `p` tags even when the composer text
/// contains no `@mention`. Agent harnesses and human notification subscriptions
/// both rely on those tags.
///
/// Mirrors desktop `messageMentionPubkeys` in
/// `desktop/src/features/messages/lib/messageMentionPubkeys.ts`.
List<String> messageMentionPubkeys({
  required bool isDm,
  required String? senderPubkey,
  required Iterable<String> explicitMentions,
  required Iterable<String> memberPubkeys,
  required Iterable<String> participantPubkeys,
}) {
  final candidates = isDm
      ? [...explicitMentions, ...memberPubkeys, ...participantPubkeys]
      : explicitMentions;

  final selfLower = senderPubkey?.toLowerCase();
  final seen = <String>{?selfLower};
  return [
    for (final pk in candidates)
      if (pk.isNotEmpty && seen.add(pk.toLowerCase())) pk,
  ];
}
