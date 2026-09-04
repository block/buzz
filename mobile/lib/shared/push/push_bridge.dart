import 'dart:async';

import 'package:nostr/nostr.dart' as nostr;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community.dart';
import '../deeplink/deep_link.dart';
import '../relay/relay_provider.dart';
import '../relay/app_lifecycle_provider.dart';
import 'push_snapshot.dart';

const _channel = MethodChannel('buzz/push');

enum BuzzPushAuthorizationStatus {
  notDetermined,
  denied,
  authorized,
  provisional,
  ephemeral,
}

typedef BuzzPushAuthorizationStatusReader =
    Future<BuzzPushAuthorizationStatus> Function();
typedef BuzzPushNotificationSettingsOpener = Future<bool> Function();

final buzzPushAuthorizationStatusReaderProvider =
    Provider<BuzzPushAuthorizationStatusReader>((ref) {
      return readBuzzPushAuthorizationStatus;
    });

final buzzPushNotificationSettingsOpenerProvider =
    Provider<BuzzPushNotificationSettingsOpener>((ref) {
      return openBuzzPushNotificationSettings;
    });

final buzzPushAuthorizationStatusProvider =
    AsyncNotifierProvider<
      BuzzPushAuthorizationStatusNotifier,
      BuzzPushAuthorizationStatus
    >(BuzzPushAuthorizationStatusNotifier.new);

class BuzzPushAuthorizationStatusNotifier
    extends AsyncNotifier<BuzzPushAuthorizationStatus> {
  @override
  Future<BuzzPushAuthorizationStatus> build() async {
    ref.listen(appLifecycleProvider, (previous, next) {
      if (previous != AppLifecycleState.resumed &&
          next == AppLifecycleState.resumed) {
        unawaited(refresh());
      }
    });
    return ref.read(buzzPushAuthorizationStatusReaderProvider)();
  }

  Future<void> refresh() async {
    state = const AsyncLoading<BuzzPushAuthorizationStatus>();
    state = await AsyncValue.guard(
      ref.read(buzzPushAuthorizationStatusReaderProvider),
    );
  }
}

Future<BuzzPushAuthorizationStatus> readBuzzPushAuthorizationStatus() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) {
    return BuzzPushAuthorizationStatus.authorized;
  }
  final raw = await _channel.invokeMethod<String>(
    'notificationAuthorizationStatus',
  );
  return switch (raw) {
    'notDetermined' => BuzzPushAuthorizationStatus.notDetermined,
    'denied' => BuzzPushAuthorizationStatus.denied,
    'authorized' => BuzzPushAuthorizationStatus.authorized,
    'provisional' => BuzzPushAuthorizationStatus.provisional,
    'ephemeral' => BuzzPushAuthorizationStatus.ephemeral,
    _ => throw FormatException(
      'Native push bridge returned unknown authorization status: $raw',
    ),
  };
}

Future<bool> openBuzzPushNotificationSettings() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return false;
  return await _channel.invokeMethod<bool>('openNotificationSettings') ?? false;
}

/// Latest APNs registration state, including callbacks replayed by iOS after
/// the Flutter method channel attaches.
final apnsDeviceToken = ValueNotifier<String?>(null);
final apnsRegistrationError = ValueNotifier<String?>(null);

final pushEndpointGrants = ValueNotifier<List<BuzzPushEndpointGrant>>([]);
final pushEndpointGrantError = ValueNotifier<String?>(null);

/// The most recent notification response waiting for app navigation.
///
/// Native iOS buffers cold-start responses until Dart asks for them. This
/// notifier also carries warm responses into the existing deep-link pipeline.
final pendingPushNotificationLink = ValueNotifier<MessageDeepLink?>(null);

MessageDeepLink? _pushNotificationLink(Object? arguments) {
  if (arguments is! Map) return null;
  final eventId = arguments['eventId'];
  final communityId = arguments['communityId'];
  final channelId = arguments['channelId'];
  if (eventId is! String ||
      eventId.isEmpty ||
      communityId is! String ||
      communityId.isEmpty ||
      channelId is! String ||
      channelId.isEmpty) {
    return null;
  }
  return MessageDeepLink(
    communityId: communityId,
    channelId: channelId,
    messageId: eventId,
  );
}

/// Pulls a notification response that arrived before the Flutter method
/// handler was installed.
Future<void> syncPendingBuzzPushNotificationResponse() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    final arguments = await _channel.invokeMapMethod<dynamic, dynamic>(
      'takePendingNotificationResponse',
    );
    final link = _pushNotificationLink(arguments);
    if (link != null) pendingPushNotificationLink.value = link;
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

/// Starts the independent iOS notification-authorization and APNs-registration
/// requests. Display authorization is intentionally not returned or persisted:
/// APNs registration and enrollment remain valid while display is denied.
Future<void> startBuzzPushRegistration() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('startRegistration');
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

class BuzzPushEndpointGrant {
  final String relayOrigin;
  final String relayPubkey;
  final String installationId;
  final String endpointGrant;
  final String endpointHash;
  final String appProfile;
  final int endpointEpoch;
  final int generation;
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
    required this.expiresAt,
  });

  factory BuzzPushEndpointGrant.fromMap(Map<dynamic, dynamic> map) {
    final generation = map['generation'] as int;
    return BuzzPushEndpointGrant(
      relayOrigin: map['relayOrigin'] as String,
      relayPubkey: map['relayPubkey'] as String,
      installationId: map['installationId'] as String,
      endpointGrant: map['endpointGrant'] as String,
      endpointHash: map['endpointHash'] as String,
      appProfile: map['appProfile'] as String,
      endpointEpoch: map['endpointEpoch'] as int,
      generation: generation,
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

/// Enrolls the endpoint and optionally rewrites the NSE snapshot afterward.
///
/// The rewrite propagates NIP-11 `self` rotations even when the opaque grant
/// and accepted relay lease remain reusable and their generations do not move.
Future<BuzzPushEndpointGrant> enrollBuzzPush(
  String relayUrl,
  String gatewayUrl, {
  List<Community>? communitiesForSnapshotRefresh,
}) async {
  final raw = await _channel.invokeMapMethod<dynamic, dynamic>('enrollPush', {
    'relayUrl': relayUrl,
    'gatewayUrl': gatewayUrl,
  });
  if (raw == null) {
    throw StateError('Native push enrollment returned no grant.');
  }
  final grant = BuzzPushEndpointGrant.fromMap(raw);
  await readBuzzPushEndpointGrants();
  if (communitiesForSnapshotRefresh != null) {
    try {
      await registerBuzzPushCommunitySnapshot(communitiesForSnapshotRefresh);
      pushCommunitySnapshotError.value = null;
    } catch (error, stackTrace) {
      reportPushCommunitySnapshotError(error, stackTrace);
    }
  }
  return grant;
}

/// Latest failure to export the community snapshot used by the iOS
/// notification service extension. Snapshot export is push enrichment and must
/// never gate authentication or community persistence.
final pushCommunitySnapshotError = ValueNotifier<String?>(null);
final pushLeaseCleanupError = ValueNotifier<String?>(null);

void reportPushCommunitySnapshotError(Object error, StackTrace stackTrace) {
  pushCommunitySnapshotError.value = error.toString();
  debugPrint('Push community snapshot export failed: $error');
  debugPrintStack(stackTrace: stackTrace);
}

void reportPushLeaseCleanupError(Object error, StackTrace stackTrace) {
  pushLeaseCleanupError.value = error.toString();
  debugPrint('Push lease cleanup failed: $error');
  debugPrintStack(stackTrace: stackTrace);
}

Future<void> registerBuzzPushCommunitySnapshot(
  List<Community> communities,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    final snapshots = [
      for (final community in communities)
        if (community.pushNotificationsEnabled)
          BuzzPushCommunitySnapshot(
            id: community.id,
            name: community.name,
            relayUrl: community.relayUrl,
            pubkey: community.pubkey ?? pubkeyFromNsec(community.nsec),
            subscriptions: community.pushSubscriptionState.authoritative,
          ),
    ];
    final signingKeys = <String, String>{};
    for (final community in communities) {
      if (!community.pushNotificationsEnabled) continue;
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
    await _channel.invokeMethod<void>('syncPushSnapshot', {
      'section': 'communities',
      'communities': [for (final snapshot in snapshots) snapshot.toJson()],
      'signingKeys': signingKeys,
    });
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
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
      case 'notificationOpened':
        final link = _pushNotificationLink(call.arguments);
        if (link == null) return 'ignored';
        pendingPushNotificationLink.value = link;
        return 'handled';
      default:
        throw MissingPluginException('Unknown buzz/push method ${call.method}');
    }
  });
}
