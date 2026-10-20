import 'push_subscription.dart';

/// The minimum community state shared with the iOS notification extension.
class BuzzPushCommunitySnapshot {
  final String id;
  final String name;
  final String relayUrl;
  final String? pubkey;
  final List<BuzzPushSubscription> subscriptions;

  BuzzPushCommunitySnapshot({
    required this.id,
    required this.name,
    required this.relayUrl,
    this.pubkey,
    required Iterable<BuzzPushSubscription> subscriptions,
  }) : subscriptions = List.unmodifiable(subscriptions);

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'relayUrl': relayUrl,
    if (pubkey != null) 'pubkey': pubkey,
    'policies': [
      for (final subscription in subscriptions)
        {
          'filter': subscription.filter.toJson(),
          if (subscription.ignore.isNotEmpty)
            'ignore': [
              for (final filter in subscription.ignore) filter.toJson(),
            ],
          if (subscription.suppress != null)
            'suppress': subscription.suppress!.toJson(),
        },
    ],
  };

  factory BuzzPushCommunitySnapshot.fromJson(Map<String, dynamic> json) {
    return BuzzPushCommunitySnapshot(
      id: json['id'] as String,
      name: json['name'] as String,
      relayUrl: json['relayUrl'] as String,
      pubkey: json['pubkey'] as String?,
      subscriptions: [
        for (final raw in json['policies'] as List<dynamic>)
          BuzzPushSubscription.fromJson({
            ...Map<String, dynamic>.from(raw as Map),
            'class': 'default',
          }),
      ],
    );
  }
}
