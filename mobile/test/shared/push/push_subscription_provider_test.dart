import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/push/push_subscription_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  const activeID = '123e4567-e89b-42d3-a456-426614174000';
  const archivedID = '123e4567-e89b-42d3-a456-426614174001';
  const nonMemberID = '123e4567-e89b-42d3-a456-426614174002';

  test(
    'derives desired subscriptions from nsec, membership, and mute state',
    () {
      final nsec = nostr.Keys.generate().nsec;
      final subscriptions = desiredBuzzPushSubscriptions(
        community: Community.create(
          name: 'Team',
          relayUrl: 'https://relay.example.com',
          nsec: nsec,
        ),
        channels: [
          channel(activeID),
          channel(archivedID, archived: true),
          channel(nonMemberID, isMember: false),
        ],
        mutedChannelIds: const [activeID, archivedID],
      );

      expect(subscriptions, isNotNull);
      expect(subscriptions, hasLength(1));
      expect(subscriptions!.single.filter.pTags, hasLength(1));
      expect(subscriptions.single.ignore, hasLength(2));
      expect(subscriptions.single.ignore.first.kinds, buzzPushRenderableKinds);
      expect(subscriptions.single.ignore.last.hTags, [activeID]);
    },
  );

  test('returns no desired subscriptions without a signing identity', () {
    final subscriptions = desiredBuzzPushSubscriptions(
      community: Community.create(
        name: 'Team',
        relayUrl: 'https://relay.example.com',
      ),
      channels: [channel(activeID)],
      mutedChannelIds: const [],
    );

    expect(subscriptions, isNull);
  });
}

Channel channel(String id, {bool isMember = true, bool archived = false}) =>
    Channel(
      id: id,
      name: id,
      channelType: 'dm',
      visibility: 'open',
      description: '',
      createdBy: 'author',
      createdAt: DateTime(2026),
      memberCount: 1,
      isMember: isMember,
      archivedAt: archived ? DateTime(2026) : null,
    );
