import 'dart:convert';

import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip_oa_test.dart' show authTag, profile;

NostrEvent signed(
  nostr.Keys key,
  int kind,
  Object content, {
  int time = 100,
  List<List<String>> tags = const [],
}) => NostrEvent.fromJson(
  nostr.Event.from(
    kind: kind,
    content: content is String ? content : jsonEncode(content),
    secretKey: key.secret,
    createdAt: time,
    tags: tags,
  ).toMap(),
);

class PolicySession extends RelaySessionNotifier {
  PolicySession(this.events, {this.failPolicy = false});
  final List<NostrEvent> events;
  final bool failPolicy;
  final queries = <NostrFilter>[];
  @override
  Future<String> fetchRelaySelf() async =>
      events.firstWhere((e) => e.kind == 39002).pubkey;
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => events.where((e) => e.kind == 10100).toList();
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    expect(filters.length, lessThanOrEqualTo(10));
    queries.addAll(filters);
    if (failPolicy && filters.any((f) => f.kinds.contains(30177))) {
      throw StateError('policy read failed');
    }
    return events
        .where(
          (event) => filters.any(
            (f) =>
                f.kinds.contains(event.kind) &&
                f.authors!.contains(event.pubkey) &&
                (f.tags['#d'] == null ||
                    f.tags['#d']!.contains(event.getTagValue('d'))),
          ),
        )
        .toList();
  }
}

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final stranger = nostr.Keys.generate();
  final owned = profile(agent, [authTag(owner, agent.public)]);
  final runtime = signed(agent, 10100, {
    'name': 'runtime',
    'respond_to': 'anyone',
    'channel_ids': ['channel'],
  });
  NostrEvent policy(Object content, {int time = 100, nostr.Keys? author}) =>
      signed(
        author ?? owner,
        30177,
        content,
        time: time,
        tags: [
          ['d', agent.public],
        ],
      );
  final allow = policy({
    'name': 'Agent',
    'parallelism': 1,
    'respond_to': 'owner-only',
  });

  Future<List<AgentDirectoryEntry>> directory(
    List<NostrEvent> events, {
    bool failPolicy = false,
    void Function(PolicySession)? inspect,
  }) async {
    final session = PolicySession([
      ...events,
      signed(
        owner,
        39002,
        '',
        tags: [
          ['d', 'channel'],
          ['p', owner.public],
          ['p', agent.public, '', 'bot'],
        ],
      ),
    ], failPolicy: failPolicy);
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        myPubkeyProvider.overrideWithValue(owner.public),
      ],
    );
    try {
      final result = await container.read(agentDirectoryProvider.future);
      inspect?.call(session);
      return result;
    } finally {
      container.dispose();
    }
  }

  test(
    'production directory reads exact authenticated owner coordinates',
    () async {
      final agents = await directory(
        [runtime, owned, allow],
        inspect: (session) {
          final query = session.queries.singleWhere(
            (q) => q.kinds.contains(30177) && q.tags.containsKey('#d'),
          );
          expect(query.authors, [owner.public]);
          expect(query.tags, {
            '#d': [agent.public],
          });
          expect(query.limit, 1);
        },
      );
      expect(agents.single.ownerPubkey, owner.public);
      expect(agents.single.respondTo, 'owner-only');
      expect(agents.single.displayName, 'Agent');
      expect(agentIsSharedWithUser(agents.single, {}, owner.public), isTrue);
      expect(
        agentIsSharedWithUser(agents.single, {'channel'}, stranger.public),
        isFalse,
      );
    },
  );

  test(
    'latest malformed policy reserves deny-all, never runtime/older fallback',
    () async {
      for (final body in [
        '',
        '{}',
        {'name': 'Agent', 'parallelism': 1, 'respond_to': 'unknown'},
        {
          'name': 'Agent',
          'parallelism': 1,
          'respond_to': 'anyone',
          'respond_to_allowlist': [3],
        },
        {'name': 'Agent', 'parallelism': -1, 'respond_to': 'anyone'},
      ]) {
        final bad = policy(body, time: 101);
        for (final policies in [
          [allow, bad],
          [bad, allow],
        ]) {
          final result = await directory([runtime, owned, ...policies]);
          expect(result.single.respondTo, 'nobody');
          expect(
            agentIsSharedWithUser(result.single, {'channel'}, owner.public),
            isFalse,
          );
        }
      }
    },
  );

  test(
    'same-second policy ties are independent of response ordering',
    () async {
      final deny = policy({
        'name': 'Agent',
        'parallelism': 1,
        'respond_to': 'nobody',
      });
      for (final policies in [
        [allow, deny],
        [deny, allow],
      ]) {
        expect(
          (await directory([runtime, owned, ...policies])).single.respondTo,
          allow.id.compareTo(deny.id) < 0 ? 'owner-only' : 'nobody',
        );
      }
    },
  );

  test(
    'tampered authenticated policy cannot revive runtime permission',
    () async {
      final tampered = NostrEvent.fromJson({
        ...allow.toJson(),
        'content': '{}',
      });
      expect(
        (await directory([runtime, owned, tampered])).single.respondTo,
        'nobody',
      );
    },
  );

  test(
    'foreign policy cannot override a headless runtime; revoked owner is not revived',
    () async {
      final foreign = policy({}, author: stranger);
      expect(
        (await directory([runtime, owned, foreign])).single.respondTo,
        'anyone',
      );
      final revoked = profile(agent, [], createdAt: 101);
      final result = await directory([runtime, owned, revoked, allow]);
      expect(result.single.ownerPubkey, isNull);
      expect(result.single.respondTo, 'anyone'); // OSS headless compatibility.
    },
  );

  test(
    'policy read failure propagates instead of falling back to runtime',
    () async {
      await expectLater(
        directory([runtime, owned, allow], failPolicy: true),
        throwsStateError,
      );
    },
  );

  test('latest tampered runtime never revives older valid runtime', () async {
    final bad = NostrEvent.fromJson({...runtime.toJson(), 'created_at': 102});
    expect(await directory([runtime, bad]), isEmpty);
  });

  test(
    'policy denial applies to member, non-member and owned search candidates',
    () async {
      final deny = policy({
        'name': 'Agent',
        'parallelism': 1,
        'respond_to': 'nobody',
      });
      final agents = await directory([runtime, owned, deny]);
      for (final members in [
        <ChannelMember>[],
        [
          ChannelMember(
            pubkey: agent.public,
            role: 'bot',
            joinedAt: DateTime(2026),
          ),
        ],
      ]) {
        final candidates = buildMentionCandidates(
          members: members,
          relayAgents: agents,
          sharedChannelIds: {'channel'},
          userCache: {},
          ownerByAgentPubkey: {agent.public: owner.public},
          currentPubkey: owner.public,
          searchResults: [
            UserProfile(pubkey: agent.public, ownerPubkey: owner.public),
          ],
        );
        expect(candidates, isEmpty);
      }
    },
  );

  test('owner is allowed by each supported mode except nobody', () async {
    for (final mode in ['owner-only', 'allowlist', 'anyone', 'nobody']) {
      final agents = await directory([
        runtime,
        owned,
        policy({'name': 'Agent', 'parallelism': 1, 'respond_to': mode}),
      ]);
      expect(
        agentIsSharedWithUser(agents.single, {}, owner.public),
        mode != 'nobody',
      );
    }
  });
}
