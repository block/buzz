import 'dart:convert';

import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip_oa_test.dart' show authTag, profile;
import 'agent_policy_test.dart' show PolicySession, signed;

class _ReadSession extends PolicySession {
  _ReadSession(super.events, this.authority);
  final String authority;
  void Function(List<NostrFilter>)? onQuery;
  List<NostrEvent> Function(List<NostrFilter>)? page;
  @override
  Future<String> fetchRelaySelf() async => authority;
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    onQuery?.call(filters);
    if (page != null && filters.first.kinds.contains(39002)) {
      return page!(filters);
    }
    return super.queryRelay(filters);
  }
}

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final relay = nostr.Keys.generate();
  final other = nostr.Keys.generate();
  final owned = profile(agent, [authTag(owner, agent.public)]);
  final policy = signed(
    owner,
    30177,
    {'name': 'Agent', 'parallelism': 1, 'respond_to': 'anyone'},
    tags: [
      ['d', agent.public],
    ],
  );
  final runtime = signed(agent, 10100, {
    'name': 'Agent',
    'respond_to': 'anyone',
    'channel_ids': ['forged-channel'],
  });
  NostrEvent members({
    String channel = 'room',
    nostr.Keys? signer,
    int time = 100,
    bool includeAgent = true,
    bool includeViewer = true,
  }) => signed(
    signer ?? relay,
    39002,
    '',
    time: time,
    tags: [
      ['d', channel],
      if (includeViewer) ['p', owner.public],
      if (includeAgent) ['p', agent.public, '', 'member'],
    ],
  );

  Future<List<AgentDirectoryEntry>> read(
    _ReadSession session, {
    bool Function()? isCurrent,
    String? destination = 'room',
  }) async {
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    container.read(relaySessionProvider);
    try {
      return await readAgentAuthorization(
        session,
        {agent.public},
        viewer: owner.public,
        channelId: destination,
        isCurrent: isCurrent ?? () => true,
      );
    } finally {
      container.dispose();
    }
  }

  test(
    'owned agent needs no runtime and ordinary membership role is sufficient',
    () async {
      final session = _ReadSession([owned, policy, members()], relay.public);
      final result = await read(session);
      expect(result.single.channelIds, ['room']);
      final filter = session.queries.first;
      expect(filter.authors, [relay.public]);
      expect(filter.tags, {
        '#p': [owner.public],
        '#d': ['room'],
      });
      expect(
        session.queries.where((q) => q.kinds.contains(10100)).single.authors,
        [agent.public],
      );
    },
  );

  test(
    'runtime cannot assert membership; exact destination excludes other rooms',
    () async {
      final result = await read(
        _ReadSession([
          owned,
          policy,
          runtime,
          members(channel: 'elsewhere'),
        ], relay.public),
      );
      expect(result.single.channelIds, isEmpty);
    },
  );

  test(
    'only relay self signature is authority; latest removals do not fall back',
    () async {
      for (final snapshots in [
        [members(signer: other)],
        [
          NostrEvent.fromJson({...members().toJson(), 'content': 'tampered'}),
        ],
        [members(), members(time: 101, includeAgent: false)],
        [members(), members(time: 101, includeViewer: false)],
      ]) {
        final result = await read(
          _ReadSession([owned, policy, ...snapshots], relay.public),
        );
        expect(result.single.channelIds, isEmpty);
      }
    },
  );

  test('same-second membership tie is deterministic', () async {
    final yes = members();
    final no = members(includeAgent: false);
    for (final snapshots in [
      [yes, no],
      [no, yes],
    ]) {
      final result = await read(
        _ReadSession([owned, policy, ...snapshots], relay.public),
      );
      expect(result.single.channelIds.isNotEmpty, yes.id.compareTo(no.id) < 0);
    }
  });

  test('scope changes stop subsequent policy queries and results', () async {
    var current = true;
    final session = _ReadSession([owned, policy, members()], relay.public);
    session.onQuery = (filters) {
      if (filters.any((f) => f.kinds.contains(0))) current = false;
    };
    await expectLater(
      read(session, isCurrent: () => current),
      throwsStateError,
    );
    expect(session.queries.any((q) => q.kinds.contains(30177)), isFalse);
  });

  test(
    'membership failure propagates, not an authoritative empty result',
    () async {
      final session = _ReadSession([owned, policy], relay.public);
      session.onQuery = (filters) {
        if (filters.first.kinds.contains(39002)) {
          throw StateError('query unavailable');
        }
      };
      await expectLater(read(session), throwsStateError);
    },
  );

  test(
    'membership pagination advances equal-time cursor and rejects stalled pages',
    () async {
      final first = members(channel: 'a');
      final last = members(channel: 'z');
      final session = _ReadSession([owned, policy], relay.public);
      var calls = 0;
      session.page = (filters) {
        calls++;
        if (calls == 1) return List.filled(500, first);
        expect(filters.single.until, first.createdAt);
        expect(filters.single.extensions['before_id'], first.id);
        return [last];
      };
      final result = await read(session, destination: null);
      expect(result.single.channelIds.toSet(), {'a', 'z'});
      calls = 0;
      final stalled = _ReadSession([owned, policy], relay.public)
        ..page = (_) => List.filled(500, first);
      await expectLater(read(stalled), throwsStateError);
    },
  );

  test(
    'NIP11 uses self, preserves host and propagates unavailable authority',
    () async {
      var body = jsonEncode({
        'self': relay.public.toUpperCase(),
        'pubkey': other.public,
      });
      var status = 200;
      final client = MockClient((request) async {
        expect(request.url.toString(), 'https://tenant.example/');
        expect(request.headers['Accept'], 'application/nostr+json');
        return http.Response(body, status);
      });
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_Config.new),
          relaySessionProvider.overrideWith(
            () => RelaySessionNotifier(httpClient: client),
          ),
        ],
      );
      try {
        final session = container.read(relaySessionProvider.notifier);
        expect(await session.fetchRelaySelf(), relay.public);
        for (final invalid in [
          jsonEncode({'pubkey': relay.public}),
          '{}',
          'not json',
          '{"self":"bad"}',
        ]) {
          body = invalid;
          await expectLater(session.fetchRelaySelf(), throwsFormatException);
        }
        status = 503;
        await expectLater(
          session.fetchRelaySelf(),
          throwsA(isA<RelayException>()),
        );
      } finally {
        container.dispose();
      }
    },
  );
}

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://tenant.example/');
}
