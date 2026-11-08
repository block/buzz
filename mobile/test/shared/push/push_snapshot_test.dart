import 'package:buzz/shared/push/push_snapshot.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('push community snapshot carries flattened resolution policies', () {
    final subscription = buildDesiredBuzzPushSubscriptions(
      myPubkey: 'a' * 64,
    ).single;
    final snapshot = BuzzPushCommunitySnapshot(
      id: 'community',
      name: 'Team',
      relayUrl: 'https://relay.example.com',
      pubkey: 'a' * 64,
      subscriptions: [subscription],
    );

    final decoded = BuzzPushCommunitySnapshot.fromJson(snapshot.toJson());

    expect(decoded.toJson(), snapshot.toJson());
    expect(decoded.subscriptions, hasLength(1));
  });
}
