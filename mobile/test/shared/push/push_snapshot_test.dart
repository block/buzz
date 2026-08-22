import 'package:buzz/shared/push/push_snapshot.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('push community snapshot carries explicit subscription authority', () {
    final subscription = buildDesiredBuzzPushSubscriptions(
      myPubkey: 'a' * 64,
    ).single;
    final snapshot = BuzzPushCommunitySnapshot(
      id: 'community',
      name: 'Team',
      relayUrl: 'https://relay.example.com',
      pubkey: 'a' * 64,
      pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
        desired: [subscription],
      ),
    );

    final decoded = BuzzPushCommunitySnapshot.fromJson(snapshot.toJson());

    expect(decoded.toJson(), snapshot.toJson());
    expect(
      decoded.pushSubscriptionState.authority,
      BuzzPushLeaseSubscriptionAuthority.desired,
    );
  });
}
