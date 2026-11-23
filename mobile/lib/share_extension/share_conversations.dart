import 'dart:convert';

import '../shared/relay/nostr_models.dart';
import 'share_bridge.dart';

/// A conversation the share can go to.
class ShareConversation {
  const ShareConversation({
    required this.id,
    required this.name,
    required this.isDm,
    required this.recipients,
    required this.lastActivity,
  });

  final String id;
  final String name;
  final bool isDm;

  /// Everyone else in a DM, who get `p` tags so they are notified.
  final List<String> recipients;
  final int lastActivity;
}

/// The picker's rows: the Bestie DM (when the app has published the Bestie)
/// and then every conversation, newest activity first.
typedef ShareDestinations = ({
  ShareConversation? bestie,
  List<ShareConversation> recent,
});

/// Reads the account's conversations straight from the relay, as a separate
/// device would.
Future<ShareDestinations> loadShareDestinations(
  ShareBridge bridge,
  SharePayload payload,
) async {
  final me = payload.community!.pubkey;
  final memberships = await bridge.query([
    NostrFilter(
      kinds: const [39002],
      tags: {
        '#p': [me],
      },
      limit: 500,
    ),
  ]);
  final ids = {
    for (final event in memberships) ?event.getTagValue('d'),
  }.toList();
  if (ids.isEmpty) return (bestie: null, recent: const <ShareConversation>[]);
  final results = await Future.wait([
    bridge.query([
      NostrFilter(kinds: const [39000], tags: {'#d': ids}, limit: ids.length),
    ]),
    bridge.query([
      NostrFilter(
        kinds: const [EventKind.streamMessage],
        tags: {'#h': ids},
        limit: 500,
      ),
    ]),
  ]);
  final latest = <String, NostrEvent>{};
  for (final event in results[0]) {
    final id = event.getTagValue('d');
    if (id == null) continue;
    if ((latest[id]?.createdAt ?? -1) < event.createdAt) latest[id] = event;
  }
  final activity = <String, int>{};
  for (final event in results[1]) {
    final id = event.getTagValue('h');
    if (id == null) continue;
    if ((activity[id] ?? 0) < event.createdAt) activity[id] = event.createdAt;
  }
  final channels = [
    for (final event in latest.values)
      (data: ChannelData.fromEvent(event), createdAt: event.createdAt),
  ].where((c) => !c.data.isArchived).toList();

  final others = {
    for (final c in channels)
      if (c.data.channelType == 'dm')
        for (final p in c.data.participantPubkeys)
          if (p.toLowerCase() != me) p.toLowerCase(),
  }.toList();
  final names = <String, String>{};
  if (others.isNotEmpty) {
    for (final profile in await bridge.query([
      NostrFilter(kinds: const [0], authors: others, limit: others.length),
    ])) {
      try {
        final json = jsonDecode(profile.content) as Map<String, dynamic>;
        final name = (json['display_name'] ?? json['name']) as String?;
        if (name != null && name.trim().isNotEmpty) {
          names[profile.pubkey.toLowerCase()] = name.trim();
        }
      } on Object {
        // An unreadable profile just leaves the short key as the name.
      }
    }
  }

  final conversations = [
    for (final c in channels)
      () {
        final isDm = c.data.channelType == 'dm';
        final recipients = isDm
            ? [
                for (final p in c.data.participantPubkeys)
                  if (p.toLowerCase() != me) p.toLowerCase(),
              ]
            : const <String>[];
        return ShareConversation(
          id: c.data.id,
          name: isDm
              ? recipients.map((p) => names[p] ?? p.substring(0, 8)).join(', ')
              : c.data.name,
          isDm: isDm,
          recipients: recipients,
          lastActivity: activity[c.data.id] ?? c.createdAt,
        );
      }(),
  ]..sort((a, b) => b.lastActivity.compareTo(a.lastActivity));

  final bestiePubkey = payload.bestie?.pubkey.toLowerCase();
  final bestie = bestiePubkey == null
      ? null
      : conversations
            .where(
              (c) =>
                  c.isDm &&
                  c.recipients.length == 1 &&
                  c.recipients.single == bestiePubkey,
            )
            .firstOrNull;
  return (
    bestie: bestie,
    recent: [
      for (final c in conversations)
        if (c.id != bestie?.id) c,
    ],
  );
}
