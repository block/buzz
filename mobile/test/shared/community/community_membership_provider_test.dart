import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const owner =
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  const admin =
      'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
  const member =
      'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

  test('parses Buzz and legacy membership tags with roles', () {
    final snapshot = communityMembershipFromEvents([
      _event(
        createdAt: 2,
        tags: [
          ['member', owner, 'owner'],
          ['member', admin.toUpperCase(), 'admin'],
          ['p', member, 'wss://relay.example.com', 'member'],
          ['member', 'not-a-pubkey', 'owner'],
          ['member', owner, 'member'],
        ],
      ),
    ]);

    expect(snapshot.snapshotFound, isTrue);
    expect(snapshot.members, hasLength(3));
    expect(snapshot.roleFor(owner), CommunityMemberRole.owner);
    expect(snapshot.roleFor(admin), CommunityMemberRole.admin);
    expect(snapshot.roleFor(member), CommunityMemberRole.member);
    expect(snapshot.pubkeys, {owner, admin, member});
  });

  test('uses the latest membership snapshot', () {
    final snapshot = communityMembershipFromEvents([
      _event(
        createdAt: 1,
        tags: [
          ['member', owner, 'owner'],
        ],
      ),
      _event(
        createdAt: 2,
        tags: [
          ['member', owner, 'admin'],
        ],
      ),
    ]);

    expect(snapshot.roleFor(owner), CommunityMemberRole.admin);
  });

  test('fails closed when no snapshot is available', () {
    final snapshot = communityMembershipFromEvents(const []);

    expect(snapshot.snapshotFound, isFalse);
    expect(snapshot.members, isEmpty);
    expect(canManageCommunityInvites(snapshot.roleFor(owner)), isFalse);
  });
}

NostrEvent _event({required int createdAt, required List<List<String>> tags}) =>
    NostrEvent(
      id: '$createdAt',
      pubkey: 'relay',
      createdAt: createdAt,
      kind: EventKind.relayMembership,
      tags: tags,
      content: '',
      sig: '',
    );
