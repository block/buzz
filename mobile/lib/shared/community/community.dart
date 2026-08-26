import 'package:uuid/uuid.dart';

import '../push/push_subscription.dart';

const _uuid = Uuid();
const _sentinel = Object();

enum SensitiveActionPolicy { enabled, disabledByUser }

class Community {
  final String id;
  final String name;
  final String relayUrl;
  final String? pubkey;
  final String? nsec;
  final SensitiveActionPolicy sensitiveActionPolicy;
  final bool pushNotificationsEnabled;
  final BuzzPushLeaseSubscriptionState pushSubscriptionState;

  /// Whether invite-created starter channels still need to be recovered.
  final bool starterSetupIncomplete;
  final DateTime addedAt;

  const Community({
    required this.id,
    required this.name,
    required this.relayUrl,
    this.pubkey,
    this.nsec,
    this.sensitiveActionPolicy = SensitiveActionPolicy.disabledByUser,
    this.pushNotificationsEnabled = false,
    this.pushSubscriptionState = const BuzzPushLeaseSubscriptionState.desired(),
    this.starterSetupIncomplete = false,
    required this.addedAt,
  });

  factory Community.create({
    required String name,
    required String relayUrl,
    String? pubkey,
    String? nsec,
    SensitiveActionPolicy sensitiveActionPolicy =
        SensitiveActionPolicy.disabledByUser,
    bool starterSetupIncomplete = false,
  }) {
    return Community(
      id: _uuid.v4(),
      name: name,
      relayUrl: relayUrl,
      pubkey: pubkey,
      nsec: nsec,
      sensitiveActionPolicy: sensitiveActionPolicy,
      starterSetupIncomplete: starterSetupIncomplete,
      addedAt: DateTime.now(),
    );
  }

  Community copyWith({
    String? name,
    String? relayUrl,
    Object? pubkey = _sentinel,
    Object? nsec = _sentinel,
    SensitiveActionPolicy? sensitiveActionPolicy,
    bool? pushNotificationsEnabled,
    BuzzPushLeaseSubscriptionState? pushSubscriptionState,
    bool? starterSetupIncomplete,
  }) {
    return Community(
      id: id,
      name: name ?? this.name,
      relayUrl: relayUrl ?? this.relayUrl,
      pubkey: pubkey == _sentinel ? this.pubkey : pubkey as String?,
      nsec: nsec == _sentinel ? this.nsec : nsec as String?,
      sensitiveActionPolicy:
          sensitiveActionPolicy ?? this.sensitiveActionPolicy,
      pushNotificationsEnabled:
          pushNotificationsEnabled ?? this.pushNotificationsEnabled,
      pushSubscriptionState:
          pushSubscriptionState ?? this.pushSubscriptionState,
      starterSetupIncomplete:
          starterSetupIncomplete ?? this.starterSetupIncomplete,
      addedAt: addedAt,
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'relayUrl': relayUrl,
    if (pubkey != null) 'pubkey': pubkey,
    if (nsec != null) 'nsec': nsec,
    'sensitiveActionPolicy': sensitiveActionPolicy.name,
    'pushNotificationsEnabled': pushNotificationsEnabled,
    'pushSubscriptionState': pushSubscriptionState.toJson(),
    'starterSetupIncomplete': starterSetupIncomplete,
    'addedAt': addedAt.toIso8601String(),
  };

  factory Community.fromJson(Map<String, dynamic> json) {
    final pushNotificationsEnabled =
        json['pushNotificationsEnabled'] as bool? ?? false;
    var pushSubscriptionState = json['pushSubscriptionState'] == null
        ? const BuzzPushLeaseSubscriptionState.desired()
        : BuzzPushLeaseSubscriptionState.fromJson(
            Map<String, dynamic>.from(json['pushSubscriptionState'] as Map),
          );
    if (!pushNotificationsEnabled &&
        pushSubscriptionState.pendingTombstoneGeneration == null) {
      pushSubscriptionState = pushSubscriptionState
          .withPendingTombstoneAtCursor();
    }
    return Community(
      id: json['id'] as String,
      name: json['name'] as String,
      relayUrl: json['relayUrl'] as String,
      pubkey: json['pubkey'] as String?,
      nsec: json['nsec'] as String?,
      sensitiveActionPolicy: SensitiveActionPolicy.values.firstWhere(
        (value) => value.name == json['sensitiveActionPolicy'],
        orElse: () => SensitiveActionPolicy.disabledByUser,
      ),
      pushNotificationsEnabled: pushNotificationsEnabled,
      pushSubscriptionState: pushSubscriptionState,
      starterSetupIncomplete: json['starterSetupIncomplete'] as bool? ?? false,
      addedAt: DateTime.parse(json['addedAt'] as String),
    );
  }

  /// Derive a human-friendly community name from a relay URL.
  static String nameFromUrl(String url) {
    try {
      final host = Uri.parse(url).host;
      if (host.contains('localhost') || host == '127.0.0.1') return 'Local Dev';
      final parts = host.split('.');
      if (parts.length > 2) return parts.first;
      return host;
    } catch (_) {
      return 'Community';
    }
  }
}
