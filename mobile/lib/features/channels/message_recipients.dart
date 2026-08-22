import 'channel.dart';

/// Who an outgoing message `p`-tags, split by why it tags them.
class MessageRecipients {
  /// Typed as `@name` in the body. Marked `mention` on a reply.
  final List<String> mentions;

  /// Addressed by the *channel* rather than by the message — every other
  /// participant in a DM, tagged whether or not anyone typed their name.
  /// Never marked, because neither role is true of it.
  final List<String> addressed;

  const MessageRecipients({required this.mentions, required this.addressed});
}

/// Semantic recipients for an outgoing mobile message, split by role.
///
/// Explicit mentions are always preserved. In a DM, every current recipient is
/// also addressed with a `p` tag without inserting visible `@mentions` into the
/// composer. Non-DM channels remain explicit-only.
///
/// The two groups stay apart because a reply marks its `p` tags with the role
/// each one plays. Returning them as one list made every DM thread reply claim
/// its counterpart had been `@`-mentioned — which pierces a mute and takes a
/// slot in the mention feed ahead of a real `@you`. Mirrors
/// `messageRecipients` in `desktop/src/features/messages/lib/messageRecipients.ts`.
MessageRecipients messageRecipients({
  required Channel channel,
  required String? senderPubkey,
  required Iterable<String> explicitMentions,
  required Iterable<String> dmRecipientPubkeys,
}) {
  final sender = senderPubkey?.toLowerCase();
  final seen = <String>{?sender};

  bool take(String candidate) =>
      candidate.trim().isNotEmpty && seen.add(candidate.toLowerCase());

  final mentions = [
    for (final candidate in explicitMentions)
      if (take(candidate)) candidate.toLowerCase(),
  ];
  final addressed = [
    if (channel.isDm)
      for (final candidate in dmRecipientPubkeys)
        if (take(candidate)) candidate.toLowerCase(),
  ];

  return MessageRecipients(mentions: mentions, addressed: addressed);
}
