import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../crypto/nip_oa_test.dart' show authTag, profile;
import 'agent_policy_test.dart' show PolicySession, signed;

class _Session extends PolicySession {
  _Session(super.events, this.authority);
  final String authority;
  void Function(NostrEvent)? changed;
  void Function(RelaySubscriptionStatus)? status;
  int closed = 0;
  List<NostrEvent> Function(NostrFilter)? ownedPage;
  @override
  Future<String> fetchRelaySelf() async => authority;
  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
  }) async {
    changed = onEvent;
    status = onStatusChanged;
    return () {
      closed++;
    };
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    if (filters.singleOrNull?.kinds.contains(30177) == true &&
        filters.single.tags.isEmpty &&
        ownedPage != null) {
      return Future.value(ownedPage!(filters.single));
    }
    return super.queryRelay(filters, timeout: timeout);
  }
}

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final relay = nostr.Keys.generate();
  final owned = profile(agent, [authTag(owner, agent.public)]);
  final policy = signed(
    owner,
    30177,
    {'name': 'Remote helper', 'parallelism': 1, 'respond_to': 'owner-only'},
    tags: [
      ['d', agent.public],
    ],
  );
  ProviderContainer container(_Session session) => ProviderContainer(
    overrides: [
      relaySessionProvider.overrideWith(() => session),
      myPubkeyProvider.overrideWithValue(owner.public),
    ],
  );

  test(
    'no runtime or shared channel required; signed owner policy reaches picker',
    () async {
      final session = _Session([owned, policy], relay.public);
      final c = container(session);
      addTearDown(c.dispose);
      final entries = await c.read(agentDirectoryProvider.future);
      expect(entries.single.pubkey, agent.public);
      expect(entries.single.channelIds, isEmpty);
      final choices = buildMentionCandidates(
        members: [],
        relayAgents: entries,
        sharedChannelIds: {},
        userCache: {},
        ownerByAgentPubkey: {},
        currentPubkey: owner.public,
      );
      expect(choices.single.label, 'Remote helper');
      expect(choices.single.ownerPubkey, owner.public);
      expect(choices.single.isAgent, isTrue);
      expect(choices.single.isMember, isFalse);
      expect(session.queries.first.authors, [owner.public]);
      expect(session.queries.first.kinds, [30177]);
    },
  );

  test(
    'owner coordinate without latest valid owner proof does not expose agent',
    () async {
      final noProof = signed(agent, 0, {'name': 'revoked'}, time: 101);
      final c = container(_Session([owned, noProof, policy], relay.public));
      addTearDown(c.dispose);
      expect(await c.read(agentDirectoryProvider.future), isEmpty);
    },
  );

  test(
    'latest nobody policy retains identity but removes picker eligibility',
    () async {
      final deny = signed(
        owner,
        30177,
        {'name': 'Remote helper', 'parallelism': 1, 'respond_to': 'nobody'},
        time: 101,
        tags: [
          ['d', agent.public],
        ],
      );
      final c = container(_Session([owned, policy, deny], relay.public));
      addTearDown(c.dispose);
      final entries = await c.read(agentDirectoryProvider.future);
      expect(entries.single.respondTo, 'nobody');
      expect(
        buildMentionCandidates(
          members: [],
          relayAgents: entries,
          sharedChannelIds: {},
          userCache: {},
          ownerByAgentPubkey: {},
          currentPubkey: owner.public,
        ),
        isEmpty,
      );
    },
  );

  test(
    'live ownership changes and recovered subscription rebuild current directory',
    () async {
      final events = [owned, policy];
      final session = _Session(events, relay.public);
      final c = container(session);
      expect((await c.read(agentDirectoryProvider.future)).length, 1);
      events.remove(policy);
      session.changed!(policy);
      await Future<void>.delayed(const Duration(milliseconds: 180));
      expect(await c.read(agentDirectoryProvider.future), isEmpty);
      events.add(policy);
      session.status!(RelaySubscriptionStatus.ready);
      await Future<void>.delayed(const Duration(milliseconds: 180));
      expect((await c.read(agentDirectoryProvider.future)).length, 1);
      c.dispose();
      expect(session.closed, 1);
      session.changed!(policy); // late events cannot invalidate a retired scope
    },
  );

  test(
    'owned coordinates paginate equal-time pages and reject a stalled cursor',
    () async {
      final session = _Session([owned, policy], relay.public);
      var calls = 0;
      session.ownedPage = (filter) {
        calls++;
        if (calls == 1) return List.filled(500, policy);
        expect(filter.until, policy.createdAt);
        expect(filter.extensions['before_id'], policy.id);
        return [];
      };
      final c = container(session);
      addTearDown(c.dispose);
      expect((await c.read(agentDirectoryProvider.future)).length, 1);
      expect(calls, 2);
      final stalled = _Session([owned, policy], relay.public)
        ..ownedPage = (_) => List.filled(500, policy);
      final c2 = container(stalled);
      addTearDown(c2.dispose);
      await expectLater(
        c2.read(agentDirectoryProvider.future),
        throwsStateError,
      );
    },
  );
}
