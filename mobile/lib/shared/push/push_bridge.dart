import 'package:nostr/nostr.dart' as nostr;
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../community/community.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_provider.dart';
import 'push_models.dart';

const _channel = MethodChannel('buzz/push');

/// Latest APNs registration state, including callbacks replayed by iOS after
/// the Flutter method channel attaches.
final apnsDeviceToken = ValueNotifier<String?>(null);
final apnsRegistrationError = ValueNotifier<String?>(null);

final pushEndpointGrants = ValueNotifier<List<BuzzPushEndpointGrant>>([]);
final pushEndpointGrantError = ValueNotifier<String?>(null);

class BuzzPushEndpointGrant {
  final String relayOrigin;
  final String relayPubkey;
  final String installationId;
  final String endpointGrant;
  final String endpointHash;
  final String appProfile;
  final int endpointEpoch;
  final int generation;
  final int? publishedGeneration;
  final int expiresAt;

  const BuzzPushEndpointGrant({
    required this.relayOrigin,
    required this.relayPubkey,
    required this.installationId,
    required this.endpointGrant,
    required this.endpointHash,
    required this.appProfile,
    required this.endpointEpoch,
    required this.generation,
    required this.publishedGeneration,
    required this.expiresAt,
  });

  factory BuzzPushEndpointGrant.fromMap(Map<dynamic, dynamic> map) {
    final generation = map['generation'] as int;
    final publishedGeneration = map['publishedGeneration'] as int?;
    if (publishedGeneration != null &&
        (publishedGeneration <= 0 || publishedGeneration > generation)) {
      throw const FormatException(
        'Published endpoint grant generation is invalid',
      );
    }
    return BuzzPushEndpointGrant(
      relayOrigin: map['relayOrigin'] as String,
      relayPubkey: map['relayPubkey'] as String,
      installationId: map['installationId'] as String,
      endpointGrant: map['endpointGrant'] as String,
      endpointHash: map['endpointHash'] as String,
      appProfile: map['appProfile'] as String,
      endpointEpoch: map['endpointEpoch'] as int,
      generation: generation,
      publishedGeneration: publishedGeneration,
      expiresAt: map['expiresAt'] as int,
    );
  }
}

Future<List<BuzzPushEndpointGrant>> readBuzzPushEndpointGrants() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return const [];
  try {
    final raw = await _channel.invokeListMethod<dynamic>('endpointGrants');
    final grants = [
      for (final value in raw ?? const [])
        BuzzPushEndpointGrant.fromMap(value as Map<dynamic, dynamic>),
    ];
    pushEndpointGrants.value = grants;
    pushEndpointGrantError.value = null;
    return grants;
  } catch (error) {
    pushEndpointGrantError.value = error.toString();
    rethrow;
  }
}

Future<BuzzPushEndpointGrant> enrollBuzzDevPush(
  String relayUrl,
  String gatewayUrl,
) async {
  if (!kDebugMode) {
    throw UnsupportedError(
      'Development push enrollment is unavailable outside debug builds.',
    );
  }
  final raw = await _channel.invokeMapMethod<dynamic, dynamic>(
    'devEnrollPush',
    {'relayUrl': relayUrl, 'gatewayUrl': gatewayUrl},
  );
  if (raw == null) {
    throw StateError('Native development push enrollment returned no grant.');
  }
  final grant = BuzzPushEndpointGrant.fromMap(raw);
  await readBuzzPushEndpointGrants();
  return grant;
}

Future<void> markBuzzDevPushLeasePublished(BuzzPushEndpointGrant grant) async {
  if (!kDebugMode) {
    throw UnsupportedError(
      'Development push publication state is unavailable outside debug builds.',
    );
  }
  await _channel.invokeMethod<void>('devMarkPushLeasePublished', {
    'relayOrigin': grant.relayOrigin,
    'appProfile': grant.appProfile,
    'generation': grant.generation,
  });
  await readBuzzPushEndpointGrants();
}

/// Latest failure to export the community snapshot used by the iOS
/// notification service extension. Snapshot export is push enrichment and must
/// never gate authentication or community persistence.
final pushCommunitySnapshotError = ValueNotifier<String?>(null);

void reportPushCommunitySnapshotError(Object error, StackTrace stackTrace) {
  pushCommunitySnapshotError.value = error.toString();
  debugPrint('Push community snapshot export failed: $error');
  debugPrintStack(stackTrace: stackTrace);
}

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
    final signingKeys = <String, String>{};
    for (final community in communities) {
      final nsec = community.nsec;
      if (nsec == null || nsec.isEmpty) continue;
      try {
        final decoded = nostr.Nip19.decode(payload: nsec);
        if (decoded.prefix != nostr.Nip19Prefix.nsec ||
            decoded.data.length != 64) {
          continue;
        }
        signingKeys[community.id] = decoded.data;
      } catch (_) {
        // Native storage is fail-closed; malformed keys are never exported.
      }
    }
    await _channel.invokeMethod<void>('saveCommunitySnapshot', {
      'communities': [for (final snapshot in snapshots) snapshot.toJson()],
      'signingKeys': signingKeys,
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
      case 'apnsTokenChanged':
        final args = call.arguments;
        if (args is Map) {
          final token = args['token'];
          if (token is String && token.isNotEmpty) {
            apnsDeviceToken.value = token;
            apnsRegistrationError.value = null;
          }
        }
        return null;
      case 'apnsRegistrationFailed':
        final args = call.arguments;
        final message = args is Map ? args['message'] : null;
        apnsRegistrationError.value = message is String && message.isNotEmpty
            ? message
            : 'APNs registration failed';
        debugPrint('APNs registration failed: ${apnsRegistrationError.value}');
        return null;
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
