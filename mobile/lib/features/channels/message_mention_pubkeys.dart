import 'channel.dart';

/// Semantic recipients for an outgoing mobile message.
///
/// Explicit mentions are always preserved. In a DM, every current recipient
/// is also addressed with a `p` tag without inserting visible `@mentions` into
/// the composer. Non-DM channels remain explicit-only.
List<String> messageMentionPubkeys({
  required Channel channel,
  required String? senderPubkey,
  required Iterable<String> explicitMentions,
  required Iterable<String> dmRecipientPubkeys,
}) {
  final sender = senderPubkey?.toLowerCase();
  final candidates = <String>[
    ...explicitMentions,
    if (channel.isDm) ...dmRecipientPubkeys,
  ];

  final seen = <String>{?sender};
  return [
    for (final candidate in candidates)
      if (candidate.trim().isNotEmpty && seen.add(candidate.toLowerCase()))
        candidate.toLowerCase(),
  ];
}
