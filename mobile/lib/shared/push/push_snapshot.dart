import 'push_subscription.dart';

/// The minimum community state shared with the iOS notification extension.
class BuzzPushCommunitySnapshot {
  final String id;
  final String name;
  final String relayUrl;
  final String? pubkey;
  final BuzzPushLeaseSubscriptionState pushSubscriptionState;

  const BuzzPushCommunitySnapshot({
    required this.id,
    required this.name,
    required this.relayUrl,
    this.pubkey,
    required this.pushSubscriptionState,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'relayUrl': relayUrl,
    if (pubkey != null) 'pubkey': pubkey,
    'pushSubscriptionState': pushSubscriptionState.toJson(),
  };

  factory BuzzPushCommunitySnapshot.fromJson(Map<String, dynamic> json) {
    return BuzzPushCommunitySnapshot(
      id: json['id'] as String,
      name: json['name'] as String,
      relayUrl: json['relayUrl'] as String,
      pubkey: json['pubkey'] as String?,
      pushSubscriptionState: BuzzPushLeaseSubscriptionState.fromJson(
        Map<String, dynamic>.from(json['pushSubscriptionState'] as Map),
      ),
    );
  }
}
