part of 'channels_provider.dart';

/// Build a [Channel] from a kind:39000 metadata event.
///
/// [displayNames] maps lowercase participant pubkey → resolved label and is
/// used to populate [Channel.participants] for DMs so [Channel.displayLabel]
/// can render real names instead of the relay-canonical "DM" name.
Channel _channelFromMeta(
  NostrEvent event,
  ChannelData data, {
  required bool isMember,
  Map<String, String> displayNames = const {},
}) {
  final participants = data.channelType == 'dm'
      ? [
          for (final pk in data.participantPubkeys)
            displayNames[pk.toLowerCase()] ?? shortPubkey(pk),
        ]
      : const <String>[];
  return Channel(
    id: data.id,
    name: data.name,
    channelType: data.channelType,
    labels: data.labels,
    visibility: data.visibility,
    description: data.description,
    topic: data.topic,
    createdBy: data.creator ?? event.pubkey,
    createdAt: DateTime.fromMillisecondsSinceEpoch(
      event.createdAt * 1000,
      isUtc: true,
    ),
    memberCount: 0,
    lastMessageAt: null,
    // `archivedAt` doubles as both the archived-state flag and the timestamp.
    // The kind:39000 metadata only carries `["archived", "true"]`, not the
    // moment of archival, so we stamp the event's `createdAt` — that's when
    // the relay republished the metadata, which is the closest signal we have.
    archivedAt: data.isArchived
        ? DateTime.fromMillisecondsSinceEpoch(
            event.createdAt * 1000,
            isUtc: true,
          )
        : null,
    participants: participants,
    participantPubkeys: data.participantPubkeys,
    isMember: isMember,
    ttlSeconds: data.ttlSeconds,
    ttlDeadline: data.ttlDeadline,
    isUnlisted: data.isUnlisted,
    joinsAsGuest: data.joinsAsGuest,
    messagePublisher: data.messagePublisher,
  );
}
