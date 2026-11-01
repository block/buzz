import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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

  group('communityMembershipProvider caching', () {
    late _CountingRelaySession session;
    late ProviderContainer container;

    setUp(() {
      session = _CountingRelaySession(owner);
      container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          relayConfigProvider.overrideWith(_TestRelayConfigNotifier.new),
        ],
      );
      addTearDown(container.dispose);
    });

    test('does not re-query when the session state changes', () async {
      final subscription = container.listen(
        communityMembershipProvider,
        (_, _) {},
      );
      addTearDown(subscription.close);

      final first = await container.read(communityMembershipProvider.future);
      expect(first.roleFor(owner), CommunityMemberRole.owner);
      expect(session.queryCount, 1);

      session.setStatus(SessionStatus.reconnecting);
      await container.pump();
      session.setStatus(SessionStatus.disconnected);
      await container.pump();
      await container.read(communityMembershipProvider.future);

      expect(session.queryCount, 1);
      final cached = container.read(communityMembershipProvider).asData?.value;
      expect(cached?.roleFor(owner), CommunityMemberRole.owner);
    });

    test('does not re-query when a consumer remounts', () async {
      final subscription = container.listen(
        communityMembershipProvider,
        (_, _) {},
      );
      await container.read(communityMembershipProvider.future);
      expect(session.queryCount, 1);

      subscription.close();
      await container.pump();

      final remounted = container.listen(
        communityMembershipProvider,
        (_, _) {},
      );
      addTearDown(remounted.close);
      await container.read(communityMembershipProvider.future);

      expect(session.queryCount, 1);
    });

    test('re-queries on explicit invalidation and on reconnect', () async {
      final subscription = container.listen(
        communityMembershipProvider,
        (_, _) {},
      );
      addTearDown(subscription.close);
      await container.read(communityMembershipProvider.future);
      expect(session.queryCount, 1);

      container.invalidate(communityMembershipProvider);
      await container.read(communityMembershipProvider.future);
      expect(session.queryCount, 2);

      session.setStatus(SessionStatus.reconnecting);
      await container.pump();
      expect(session.queryCount, 2);
      session.setStatus(SessionStatus.connected);
      await container.pump();
      await container.read(communityMembershipProvider.future);
      expect(session.queryCount, 3);
    });
  });
}

class _TestRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      const RelayConfig(baseUrl: 'https://relay.example.com');
}

class _CountingRelaySession extends RelaySessionNotifier {
  _CountingRelaySession(this.owner);

  final String owner;
  int queryCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryCount++;
    return [
      _event(
        createdAt: queryCount,
        tags: [
          ['member', owner, 'owner'],
        ],
      ),
    ];
  }
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
