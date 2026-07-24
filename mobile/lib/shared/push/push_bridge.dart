import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../community/community.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_provider.dart';
import 'push_models.dart';

const _channel = MethodChannel('buzz/push');

Future<void> registerBuzzPushCommunitySnapshot(
  List<Community> communities,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    final snapshots = [
      for (final community in communities)
        BuzzPushCommunitySnapshot(
          id: community.id,
          name: community.name,
          relayUrl: community.relayUrl,
          pubkey: community.pubkey ?? pubkeyFromNsec(community.nsec),
        ),
    ];
    await _channel.invokeMethod<void>('saveCommunitySnapshot', {
      'communities': [for (final snapshot in snapshots) snapshot.toJson()],
    });
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

Future<BuzzPushResolution?> resolveBuzzPushPayload(
  Map<String, dynamic> arguments,
) async {
  final myPubkey = arguments['pubkey'] as String?;
  final communityName = arguments['communityName'] as String? ?? 'Buzz';
  if (myPubkey == null || myPubkey.isEmpty) return null;

  final eventPayloads = arguments['events'];
  if (eventPayloads is! List) return null;
  final events = <NostrEvent>[];
  for (final payload in eventPayloads) {
    if (payload is Map) {
      try {
        events.add(NostrEvent.fromJson(Map<String, dynamic>.from(payload)));
      } catch (_) {
        // Ignore malformed relay rows and preserve the fallback notification.
      }
    }
  }

  final profiles = <String, ProfileData>{};
  final profilePayloads = arguments['profiles'];
  if (profilePayloads is List) {
    for (final payload in profilePayloads) {
      if (payload is Map) {
        try {
          final event = NostrEvent.fromJson(Map<String, dynamic>.from(payload));
          final profile = ProfileData.fromEvent(event);
          profiles[profile.pubkey.toLowerCase()] = profile;
        } catch (_) {}
      }
    }
  }

  return resolveBuzzPushNotification(
    events: events,
    myPubkey: myPubkey,
    communityName: communityName,
    channelName: arguments['channelName'] as String?,
    profilesByPubkey: profiles,
  );
}

void installBuzzPushMethodHandler() {
  _channel.setMethodCallHandler((call) async {
    switch (call.method) {
      case 'resolveNotification':
        final args = call.arguments;
        if (args is! Map) return null;
        return (await resolveBuzzPushPayload(
          Map<String, dynamic>.from(args),
        ))?.toJson();
      default:
        throw MissingPluginException('Unknown buzz/push method ${call.method}');
    }
  });
}
