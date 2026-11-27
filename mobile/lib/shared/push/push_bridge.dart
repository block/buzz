import 'dart:async';

import 'package:nostr/nostr.dart' as nostr;
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../community/community.dart';
import '../deeplink/deep_link.dart';
import '../diagnostics/diagnostic_reporter.dart';
import '../relay/relay_provider.dart';
import 'android_push_authority.dart';
import 'android_push_gateway.dart';
import 'android_push_keystore.dart';
import 'push_snapshot.dart';

const _channel = MethodChannel('buzz/push');

/// Android's FCM token registration. It has its own channel because Android's
/// notification-open handler already owns `buzz/push`, and a channel has one
/// native handler.
const _registrationChannel = MethodChannel('buzz/push_registration');

/// The code native enrollment fails with when the push gateway's reply does
/// not match its protocol.
const _gatewayResponseInvalid = 'gateway_response_invalid';

/// Latest APNs registration state, including callbacks replayed by iOS after
/// the Flutter method channel attaches.
final apnsDeviceToken = ValueNotifier<String?>(null);
final apnsRegistrationError = ValueNotifier<String?>(null);

/// Latest FCM registration token and failure on Android. They play the role
/// of the two APNs notifiers above; a phone only ever sets one pair.
final fcmDeviceToken = ValueNotifier<String?>(null);
final fcmRegistrationError = ValueNotifier<String?>(null);

/// Why push cannot work on this phone (no hardware key attestation, no
/// Firebase, no FCM token), or null. The app works without push; settings
/// shows the reason.
final pushUnavailableReason = ValueNotifier<String?>(null);

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
    this.recipientPubkey,
    this.inSolo = false,
    this.recovered = false,
    this.mayHavePublished = false,
  });

  final String id;
  final String text;

  /// The notified message, and where tapping its notification opens.
  final MessageDeepLink link;

  /// Author of the notified message, whom the reply mentions.
  final String senderPubkey;

  /// NIP-10 thread root of the notified message; null when it is top-level.
  final String? replyRootId;

  /// The account the notification was for, which the reply must go out as,
  /// guarding against a community's credentials being replaced in place. Null
  /// for a notification an earlier build posted.
  final String? recipientPubkey;

  /// When the notified message was created, in Unix seconds.
  final int messageCreatedAt;

  /// When iOS stops waiting for the reply and reports it as not sent.
  final DateTime deadline;

  /// Whether the notification knew the message is in a Solo chat, which
  /// takes no thread replies.
  final bool inSolo;

  /// Whether an earlier launch was given this reply and never finished it.
  /// It is never sent again.
  final bool recovered;

  /// Whether that earlier launch may have published it.
  final bool mayHavePublished;

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
    final recipient = arguments['recipientPubkey'];
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
        (root != null && (root is! String || root.isEmpty)) ||
        (recipient != null && (recipient is! String || recipient.isEmpty))) {
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
      recipientPubkey: recipient as String?,
      inSolo: arguments['inSolo'] == 'true',
      recovered: arguments['recovered'] == 'true',
      mayHavePublished: arguments['publishing'] == 'true',
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

/// Tells iOS that a notification reply may be published from now on, so a
/// relaunch never drafts or resends it. Returns once that is saved, false
/// when iOS has already ended the reply, which then must not be published.
Future<bool> markBuzzNotificationReplyPublishing(String id) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return true;
  try {
    return await _channel.invokeMethod<bool>(
          'markNotificationReplyPublishing',
          id,
        ) ??
        true;
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
    return true;
  }
}

/// Reports whether a notification reply was sent, so iOS clears the
/// conversation's notifications or shows that it wasn't. [unknown] means it
/// may have gone out; [draftThreadRootId] is the thread holding its draft,
/// which the "Reply not sent" notification opens. Without [draftKept], iOS
/// shows the unsent text in a notification even for a reply an earlier
/// launch was given.
Future<void> completeBuzzNotificationReply(
  String id, {
  required bool sent,
  bool unknown = false,
  String? draftThreadRootId,
  bool draftKept = true,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('completeNotificationReply', {
      'id': id,
      'sent': sent,
      if (unknown) 'unknown': true,
      'draftThreadRootId': ?draftThreadRootId,
      if (!draftKept) 'noDraft': true,
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
  if (defaultTargetPlatform != TargetPlatform.iOS &&
      defaultTargetPlatform != TargetPlatform.android) {
    return;
  }
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
      (defaultTargetPlatform != TargetPlatform.iOS &&
          defaultTargetPlatform != TargetPlatform.android)) {
    return;
  }
  try {
    await (defaultTargetPlatform == TargetPlatform.android
            ? _registrationChannel
            : _channel)
        .invokeMethod<void>('startRegistration');
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

/// Asks Android for the current FCM token. FCM rotates it on its own, and the
/// app learns of that when it next runs. A changed token arrives through
/// [fcmDeviceToken] like the first one.
Future<void> refreshBuzzPushToken() async {
  if (!Env.pushGatewayConfigured ||
      defaultTargetPlatform != TargetPlatform.android) {
    return;
  }
  try {
    await _registrationChannel.invokeMethod<void>('refreshToken');
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  }
}

AndroidPushAuthority? _androidAuthority;

AndroidPushAuthority get _androidPushAuthority =>
    _androidAuthority ??= AndroidPushAuthority(
      keystore: const MethodChannelAndroidPushKeystore(),
      gateway: AndroidPushGatewayClient(baseUrl: Uri.parse(Env.pushGatewayUrl)),
      store: SecureAndroidPushStateStore(),
    );

@visibleForTesting
set debugAndroidPushAuthority(AndroidPushAuthority? authority) =>
    _androidAuthority = authority;

BuzzPushEndpointGrant _grantOf(AndroidPushGrant grant) => BuzzPushEndpointGrant(
  relayOrigin: grant.relayOrigin,
  relayPubkey: grant.relayPubkey,
  installationId: grant.installationId,
  endpointGrant: grant.endpointGrant,
  endpointHash: grant.endpointHash,
  endpointEpoch: grant.endpointEpoch,
  generation: grant.generation,
  expiresAt: grant.expiresAt,
);

/// Revokes what the phone's push installation holds for relays no community
/// uses any more, and the installation itself once no community remains.
/// Android only; iOS revokes only relay leases. [desiredRelays] and
/// [noCommunities] are read when the work runs.
Future<void> reconcileBuzzPushAuthority({
  required Set<String> Function() desiredRelays,
  required bool Function() noCommunities,
}) async {
  if (!Env.pushGatewayConfigured ||
      defaultTargetPlatform != TargetPlatform.android) {
    return;
  }
  await _androidPushAuthority.reconcile(
    desiredRelays: desiredRelays,
    revokeInstallation: noCommunities,
  );
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
      (defaultTargetPlatform != TargetPlatform.iOS &&
          defaultTargetPlatform != TargetPlatform.android)) {
    return const [];
  }
  try {
    final List<BuzzPushEndpointGrant> grants;
    if (defaultTargetPlatform == TargetPlatform.android) {
      grants = [
        for (final grant in await _androidPushAuthority.grants())
          _grantOf(grant),
      ];
    } else {
      final raw = await _channel.invokeListMethod<dynamic>('endpointGrants', {
        'gatewayUrl': Env.pushGatewayUrl,
      });
      grants = [
        for (final value in raw ?? const [])
          BuzzPushEndpointGrant.fromMap(value as Map<dynamic, dynamic>),
      ];
    }
    pushEndpointGrants.value = grants;
    pushEndpointGrantError.value = null;
    return grants;
  } catch (error) {
    pushEndpointGrantError.value = error.toString();
    rethrow;
  }
}

bool _wanted() => true;

/// Enrolls the endpoint and optionally rewrites the NSE snapshot afterward.
///
/// The rewrite propagates NIP-11 `self` rotations even when the opaque grant
/// and accepted relay lease remain reusable and their generations do not move.
///
/// On Android the grant comes from the Dart-side [AndroidPushAuthority], which
/// needs the relay's [relayOrigin] and push key [relayPubkey] from its
/// descriptor, where iOS native code fetches them itself.
///
/// [desiredRelays] (Android) are the relay origins whose push is on, which
/// enrollment uses to revoke delegations no relay points at any more.
///
/// [stillWanted] says whether the community still wants push. It is read before
/// each call that creates authority, and the result is null once it said no.
Future<BuzzPushEndpointGrant?> enrollBuzzPush(
  String relayUrl,
  String gatewayUrl, {
  List<Community>? communitiesForSnapshotRefresh,
  String? relayOrigin,
  String? relayPubkey,
  Set<String> Function()? desiredRelays,
  bool Function() stillWanted = _wanted,
  DiagnosticReporter diagnostics = const NoopDiagnosticReporter(),
}) async {
  final BuzzPushEndpointGrant grant;
  if (defaultTargetPlatform == TargetPlatform.android) {
    final token = fcmDeviceToken.value;
    if (token == null ||
        relayOrigin == null ||
        relayPubkey == null ||
        desiredRelays == null) {
      throw StateError('Android push enrollment needs a token and a relay.');
    }
    final AndroidPushGrant? enrolled;
    try {
      enrolled = await _androidPushAuthority.enroll(
        relayOrigin: relayOrigin,
        relayPubkey: relayPubkey,
        endpoint: token,
        stillWanted: stillWanted,
        desiredRelays: desiredRelays,
      );
    } on AndroidPushAttestationUnavailable catch (error) {
      pushUnavailableReason.value = error.reason;
      rethrow;
    } on AndroidPushGatewayResponseInvalid catch (error, stackTrace) {
      // As on iOS, only a reply the app cannot read is reported; refusals and
      // transport failures are routine.
      diagnostics.reportError(
        error,
        stackTrace,
        operation: DiagnosticOperation.pushGatewayResponse,
      );
      rethrow;
    }
    if (enrolled == null) return null;
    grant = _grantOf(enrolled);
    pushUnavailableReason.value = null;
  } else {
    if (!stillWanted()) return null;
    final Map<dynamic, dynamic>? raw;
    try {
      raw = await _channel.invokeMapMethod<dynamic, dynamic>('enrollPush', {
        'relayUrl': relayUrl,
        'gatewayUrl': gatewayUrl,
      });
    } on PlatformException catch (error, stackTrace) {
      // The gateway is called from native code. It has its own code for a
      // reply the app cannot read; every other failure (offline, refused,
      // expired, server errors) is routine and not reported.
      if (error.code == _gatewayResponseInvalid) {
        diagnostics.reportError(
          error,
          stackTrace,
          operation: DiagnosticOperation.pushGatewayResponse,
        );
      }
      rethrow;
    }
    if (raw == null) {
      throw StateError('Native push enrollment returned no grant.');
    }
    grant = BuzzPushEndpointGrant.fromMap(raw);
  }
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

/// Platforms whose native push code honors a confirmed age restriction.
bool get _ageRestrictionPlatform =>
    defaultTargetPlatform == TargetPlatform.iOS ||
    defaultTargetPlatform == TargetPlatform.android;

/// Restores notification presentation without reading community storage.
Future<void> restoreAgeRestrictedBuzzNotifications() async {
  if (!_ageRestrictionPlatform) return;
  await _channel.invokeMethod<void>('restoreAgeRestrictedNotifications');
}

/// Removes notifications rendered before a confirmed age restriction.
Future<void> purgeAgeRestrictedBuzzNotifications() async {
  if (!_ageRestrictionPlatform) return;
  await _channel.invokeMethod<void>('purgeAgeRestrictedNotifications');
}

Future<void> registerBuzzPushCommunitySnapshot(
  List<Community> communities,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    // An unconfirmed opt-out stays in: until the relay confirms it,
    // notifications may still arrive, and the extension must resolve them.
    bool exported(Community community) =>
        community.pushNotificationsEnabled ||
        community.pushSubscriptionState.pendingTombstoneGeneration != null;
    final snapshots = [
      for (final community in communities)
        if (exported(community))
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
      if (!exported(community)) continue;
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
  _registrationChannel.setMethodCallHandler((call) async {
    switch (call.method) {
      case 'fcmTokenChanged':
        final args = call.arguments;
        if (args is Map) {
          final token = args['token'];
          if (token is String && token.isNotEmpty) {
            fcmDeviceToken.value = token;
            // A token ends an earlier failure to get one, but not an
            // attestation failure, which enrollment owns.
            if (pushUnavailableReason.value == fcmRegistrationError.value) {
              pushUnavailableReason.value = null;
            }
            fcmRegistrationError.value = null;
          }
        }
        return null;
      case 'fcmRegistrationFailed':
        final args = call.arguments;
        final message = args is Map ? args['message'] : null;
        final reason = message is String && message.isNotEmpty
            ? message
            : 'FCM registration failed';
        final previous = fcmRegistrationError.value;
        fcmRegistrationError.value = reason;
        // A held token still delivers, so a failed refresh is transient. Never
        // overwrite another reason, such as attestation, which enrollment owns.
        final shown = pushUnavailableReason.value;
        if (fcmDeviceToken.value == null &&
            (shown == null || shown == previous)) {
          pushUnavailableReason.value = reason;
        }
        debugPrint('FCM registration failed: $reason');
        return null;
      default:
        throw MissingPluginException('Unexpected ${call.method}');
    }
  });
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
