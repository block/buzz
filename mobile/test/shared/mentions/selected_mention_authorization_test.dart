import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip_oa_test.dart' show authTag, profile;
import 'agent_policy_test.dart' show PolicySession, signed;

class _Session extends PolicySession {
  _Session(super.events, this.authority, {super.failPolicy});
  final String authority;
  void Function(List<NostrFilter>)? onQuery;
  @override
  Future<String> fetchRelaySelf() async => authority;
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    onQuery?.call(filters);
    return super.queryRelay(filters, timeout: timeout);
  }
}

void main() {
  final viewer = nostr.Keys.generate();
  final person = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final relay = nostr.Keys.generate();
  final owned = profile(agent, [authTag(viewer, agent.public)]);
  final runtime = signed(agent, 10100, {'respond_to': 'anyone'});
  NostrEvent roster({String? role, bool personMember = true, int time = 100}) =>
      signed(
        relay,
        39002,
        '',
        time: time,
        tags: [
          ['d', 'room'],
          ['p', viewer.public],
          if (personMember) ['p', person.public, '', 'member'],
          if (role != null) ['p', agent.public, '', role],
        ],
      );
  NostrEvent policy(String mode, {int time = 100}) => signed(
    viewer,
    30177,
    {'name': 'Agent', 'parallelism': 1, 'respond_to': mode},
    time: time,
    tags: [
      ['d', agent.public],
    ],
  );

  Future<Map<String, SelectedMentionAuthorization>> read(
    List<NostrEvent> events, {
    Set<String> prior = const {},
    bool Function()? current,
    _Session? session,
    Set<String>? keys,
  }) async {
    final source = session ?? _Session(events, relay.public);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => source)],
    );
    container.read(relaySessionProvider);
    try {
      return await readSelectedMentionAuthorization(
        source,
        keys ?? {person.public, agent.public},
        viewer: viewer.public,
        channelId: 'room',
        priorAgentKeys: prior,
        isCurrent: current ?? () => true,
      );
    } finally {
      container.dispose();
    }
  }

  test(
    'exact total map preserves ordinary member and nonmember flows',
    () async {
      for (final member in [true, false]) {
        final result = await read([roster(personMember: member)]);
        expect(result.keys.toSet(), {person.public, agent.public});
        final ordinary = result[person.public]!;
        expect(ordinary.kind, SelectedMentionKind.ordinary);
        expect(ordinary.requiresAgentAuthorization, isFalse);
        expect(ordinary.isMember, member);
        expect(ordinary.invitationRole, 'member');
        expect(() => result.clear(), throwsUnsupportedError);
      }
    },
  );

  test(
    'saved false cannot exclude fresh agent or select member role',
    () async {
      final result = await read([
        roster(role: 'member'),
        owned,
        policy('anyone'),
      ]);
      final fresh = result[agent.public]!;
      expect(fresh.kind, SelectedMentionKind.agent);
      expect(fresh.requiresAgentAuthorization, isTrue);
      expect(fresh.invitationRole, 'bot');
      expect(fresh.isMember, isTrue);
      expect(fresh.agent!.ownerPubkey, viewer.public);
      expect(fresh.agent!.channelIds, ['room']);
      expect(result[person.public]!.kind, SelectedMentionKind.ordinary);
    },
  );

  test(
    'saved true with lost provenance remains unresolved even as member',
    () async {
      final result = await read(
        [roster(role: 'member'), owned, profile(agent, [], createdAt: 101)],
        prior: {agent.public},
      );
      final lost = result[agent.public]!;
      expect(lost.kind, SelectedMentionKind.unresolvedAgent);
      expect(lost.isMember, isTrue);
      expect(lost.requiresAgentAuthorization, isTrue);
      expect(lost.agent, isNull);
      expect(lost.invitationRole, isNull);
    },
  );

  test(
    'missing and revoked owner policy deny without runtime fallback',
    () async {
      for (final policies in <List<NostrEvent>>[
        [],
        [policy('anyone'), policy('nobody', time: 101)],
      ]) {
        final result = await read([
          roster(role: 'bot'),
          owned,
          runtime,
          ...policies,
        ]);
        final fresh = result[agent.public]!;
        expect(fresh.kind, SelectedMentionKind.agent);
        expect(fresh.agent!.respondTo, 'nobody');
        expect(fresh.agent!.ownerPubkey, viewer.public);
      }
    },
  );

  test(
    'known runtime agent without membership never becomes ordinary',
    () async {
      final result = await read([roster(), runtime]);
      expect(result[agent.public]!.kind, SelectedMentionKind.agent);
      expect(result[agent.public]!.agent!.channelIds, isEmpty);
      expect(result[agent.public]!.isMember, isFalse);
    },
  );

  test(
    'headless bot requires runtime policy; malformed OA is unresolved',
    () async {
      final bot = (await read([roster(role: 'bot')]))[agent.public]!;
      expect(bot.kind, SelectedMentionKind.agent);
      expect(bot.agent!.respondTo, isNull);
      final good = (await read([roster(role: 'bot'), runtime]))[agent.public]!;
      expect(good.agent!.respondTo, 'anyone');
      expect(good.agent!.channelIds, ['room']);
      final bad = (await read([
        roster(role: 'bot'),
        runtime,
        profile(agent, [
          ['auth', viewer.public, '', 'bad'],
        ]),
      ]))[agent.public]!;
      expect(bad.kind, SelectedMentionKind.unresolvedAgent);
      expect(bad.agent, isNull);
    },
  );

  test(
    'newest roster removal wins; invalid head does not revive old member',
    () async {
      final result = await read([
        roster(role: 'bot'),
        roster(time: 101),
        runtime,
      ]);
      expect(result[agent.public]!.isMember, isFalse);
      expect(result[agent.public]!.agent!.channelIds, isEmpty);
      final invalid = NostrEvent.fromJson({
        ...roster(time: 102).toJson(),
        'content': 'tampered',
      });
      await expectLater(read([roster(), invalid]), throwsStateError);
      await expectLater(read([]), throwsStateError);
    },
  );

  test(
    'policy failures and scope retirement throw rather than partial map',
    () async {
      await expectLater(
        read(
          [],
          session: _Session([roster(), owned], relay.public, failPolicy: true),
        ),
        throwsStateError,
      );
      var current = true;
      final session = _Session([roster(), owned], relay.public)
        ..onQuery = (filters) {
          if (filters.any((f) => f.kinds.contains(0))) current = false;
        };
      await expectLater(
        read([], session: session, current: () => current),
        throwsStateError,
      );
      expect(session.queries.any((f) => f.kinds.contains(30177)), isFalse);
    },
  );

  test('exact request bounds and denial-only taint are validated', () async {
    await expectLater(read([roster()], keys: {'not-a-key'}), throwsStateError);
    await expectLater(
      read([roster()], keys: {person.public}, prior: {agent.public}),
      throwsStateError,
    );
    final keys = {
      for (var i = 0; i < 1001; i++) i.toRadixString(16).padLeft(64, '0'),
    };
    await expectLater(read([roster()], keys: keys), throwsStateError);
  });
}
