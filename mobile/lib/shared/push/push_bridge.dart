import 'dart:async';

import 'package:nostr/nostr.dart' as nostr;
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../community/community.dart';
import '../deeplink/deep_link.dart';
import '../relay/relay_provider.dart';
import 'push_snapshot.dart';

const _channel = MethodChannel('buzz/push');

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
  final threadRootId = arguments['threadRootId'];
  if (eventId is! String ||
      eventId.isEmpty ||
      communityId is! String ||
      communityId.isEmpty ||
      channelId is! String ||
      channelId.isEmpty ||
      (threadRootId != null &&
          (threadRootId is! String || threadRootId.isEmpty))) {
    return null;
  }
  return MessageDeepLink(
    communityId: communityId,
    channelId: channelId,
    messageId: eventId,
    threadRootId: threadRootId as String?,
  );
}

/// A reply typed into an iOS notification, waiting to be sent.
@immutable
class BuzzNotificationReply {
  const BuzzNotificationReply({
    required this.id,
    required this.text,
    required this.link,
    required this.senderPubkey,
    required this.messageCreatedAt,
    required this.deadline,
    this.replyRootId,
  });

  final String id;
  final String text;

  /// The notified message, and where tapping its notification opens.
  final MessageDeepLink link;

  /// Author of the notified message, whom the reply mentions.
  final String senderPubkey;

  /// NIP-10 thread root of the notified message; null when it is top-level.
  final String? replyRootId;

  /// When the notified message was created, in Unix seconds.
  final int messageCreatedAt;

  /// When iOS stops waiting for the reply and reports it as not sent.
  final DateTime deadline;

  static BuzzNotificationReply? fromArguments(Object? arguments) {
    if (arguments is! Map) return null;
    final link = _pushNotificationLink({
      for (final key in const [
        'eventId',
        'communityId',
        'channelId',
        'threadRootId',
      ])
        if (arguments.containsKey(key)) key: arguments[key],
    });
    final id = arguments['id'];
    final text = arguments['text'];
    final sender = arguments['senderPubkey'];
    final root = arguments['replyRootId'];
    final createdAt = switch (arguments['messageCreatedAt']) {
      final String value => int.tryParse(value),
      _ => null,
    };
    final deadline = switch (arguments['deadline']) {
      final String value => int.tryParse(value),
      _ => null,
    };
    if (link == null ||
        id is! String ||
        id.isEmpty ||
        text is! String ||
        text.trim().isEmpty ||
        sender is! String ||
        sender.isEmpty ||
        createdAt == null ||
        createdAt <= 0 ||
        deadline == null ||
        (root != null && (root is! String || root.isEmpty))) {
      return null;
    }
    return BuzzNotificationReply(
      id: id,
      text: text,
      link: link,
      senderPubkey: sender,
      messageCreatedAt: createdAt,
      deadline: DateTime.fromMillisecondsSinceEpoch(deadline),
      replyRootId: root as String?,
    );
  }
}

/// Bumped when iOS hands the app a notification reply to send.
final notificationReplyAvailable = ValueNotifier<int>(0);

/// Takes notification replies native iOS is holding for the app.
Future<List<BuzzNotificationReply>> takePendingBuzzNotificationReplies() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return const [];
  try {
    final raw = await _channel.invokeListMethod<dynamic>(
      'takePendingNotificationReplies',
    );
    return [
      for (final value in raw ?? const [])
        ?BuzzNotificationReply.fromArguments(value),
    ];
  } on MissingPluginException {
    return const [];
  }
}

/// Reports whether a notification reply was sent, so iOS clears the
/// conversation's notifications or shows that it wasn't. [unknown] means it
/// may have gone out; [draftThreadRootId] is the thread holding its draft,
/// which the "Reply not sent" notification opens.
Future<void> completeBuzzNotificationReply(
  String id, {
  required bool sent,
  bool unknown = false,
  String? draftThreadRootId,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('completeNotificationReply', {
      'id': id,
      'sent': sent,
      if (unknown) 'unknown': true,
      'draftThreadRootId': ?draftThreadRootId,
    });
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

/// Tells the iOS notification extension which community is selected; only
/// its notifications offer Reply.
Future<void> shareSelectedCommunityWithNotifications(
  String? communityId,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('setSelectedCommunity', communityId);
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
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
  if (!Env.pushGatewayConfigured ||
      defaultTargetPlatform != TargetPlatform.iOS) {
    return;
  }
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
  final int endpointEpoch;
  final int generation;
  final int expiresAt;

  const BuzzPushEndpointGrant({
    required this.relayOrigin,
    required this.relayPubkey,
    required this.installationId,
    required this.endpointGrant,
    required this.endpointHash,
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
      endpointEpoch: map['endpointEpoch'] as int,
      generation: generation,
      expiresAt: map['expiresAt'] as int,
    );
  }
}

Future<List<BuzzPushEndpointGrant>> readBuzzPushEndpointGrants() async {
  if (!Env.pushGatewayConfigured ||
      defaultTargetPlatform != TargetPlatform.iOS) {
    return const [];
  }
  try {
    final raw = await _channel.invokeListMethod<dynamic>('endpointGrants', {
      'gatewayUrl': Env.pushGatewayUrl,
    });
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

/// Restores notification presentation without reading community storage.
Future<void> restoreAgeRestrictedBuzzNotifications() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  await _channel.invokeMethod<void>('restoreAgeRestrictedNotifications');
}

/// Removes notifications rendered before a confirmed age restriction.
Future<void> purgeAgeRestrictedBuzzNotifications() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  await _channel.invokeMethod<void>('purgeAgeRestrictedNotifications');
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

/// Shares the organization account session with the iOS notification
/// extension, or removes it when [credential] is null. Failures propagate to
/// the account client, which owns retrying them. Organization relays
/// enforce NIP-FI, so the extension needs the session to mint the assertion
/// that lets it read the message a push refers to.
Future<void> shareOrganizationSessionWithNotifications({
  required Uri origin,
  required String? credential,
  required List<String> relayHosts,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('syncPushSnapshot', {
      'section': 'organizationSession',
      'origin': origin.toString(),
      'credential': credential,
      'relayHosts': relayHosts,
    });
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

/// Shares the hosted account session with the iOS notification extension, or
/// removes it when [credential] is null, like
/// [shareOrganizationSessionWithNotifications]. A hosted relay matches when its
/// host is under [communityDomain] and equal to or under one of
/// [relayDomains], as in `HostedRelayAuth`.
Future<void> shareHostedSessionWithNotifications({
  required Uri origin,
  required String? credential,
  required List<String> relayDomains,
  required String communityDomain,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('syncPushSnapshot', {
      'section': 'hostedSession',
      'origin': origin.toString(),
      'credential': credential,
      'relayDomains': relayDomains,
      'communityDomain': communityDomain,
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
      case 'notificationReplyAvailable':
        notificationReplyAvailable.value++;
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
