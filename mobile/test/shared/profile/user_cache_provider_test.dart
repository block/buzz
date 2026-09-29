import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/profile_event_parser.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

void main() {
  test('preload reports a profile batch failure', () async {
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(_FailingProfileSession.new),
      ],
    );
    addTearDown(container.dispose);

    final succeeded = await container.read(userCacheProvider.notifier).preload(
      const ['agent'],
    );

    expect(succeeded, isFalse);
  });

  test('refresh queries profiles that are already cached', () async {
    final session = _RecordingProfileSession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    cache.cacheProfileEvent(
      _profileEvent(id: 'cached-profile', createdAt: 1, name: 'Cached Human'),
    );

    final succeeded = await cache.refresh(const ['AGENT']);

    expect(succeeded, isTrue);
    expect(session.requestedFilter?.kinds, const [0]);
    expect(session.requestedFilter?.authors, const ['agent']);
    expect(session.requestedFilter?.limit, 1);
  });

  test('older refresh cannot overwrite a newer live profile', () async {
    final refreshCompleter = Completer<List<NostrEvent>>();
    final session = _RecordingProfileSession(result: refreshCompleter.future);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    final owner = nostr.Keys.generate();
    final agent = nostr.Keys.generate();
    final refresh = cache.refresh([agent.public]);

    cache.cacheProfileEvent(
      _profileEvent(
        id: 'newer-agent',
        pubkey: agent.public,
        createdAt: 2,
        name: 'Agent',
        tags: [_authTag(owner, agent.public)],
      ),
    );
    refreshCompleter.complete([
      _profileEvent(
        id: 'older-human',
        pubkey: agent.public,
        createdAt: 1,
        name: 'Human',
      ),
    ]);

    expect(await refresh, isTrue);
    expect(cache.state[agent.public]?.displayName, 'Agent');
    expect(cache.state[agent.public]?.ownerPubkey, owner.public);
  });

  test('newer refresh can remove obsolete owner attribution', () async {
    final owner = nostr.Keys.generate();
    final agent = nostr.Keys.generate();
    final session = _RecordingProfileSession(
      result: Future.value([
        _profileEvent(
          id: 'newer-human',
          pubkey: agent.public,
          createdAt: 2,
          name: 'Human',
        ),
      ]),
    );
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    cache.cacheProfileEvent(
      _profileEvent(
        id: 'older-agent',
        pubkey: agent.public,
        createdAt: 1,
        name: 'Agent',
        tags: [_authTag(owner, agent.public)],
      ),
    );

    expect(await cache.refresh([agent.public]), isTrue);
    expect(cache.state[agent.public]?.displayName, 'Human');
    expect(cache.state[agent.public]?.ownerPubkey, isNull);
  });

  test('non-profile history cannot poison profile order', () async {
    final session = _RecordingProfileSession(
      results: [
        Future.value([
          _profileEvent(
            id: 'non-profile-newer',
            createdAt: 3,
            name: 'Ignored',
            kind: 1,
          ),
        ]),
        Future.value([
          _profileEvent(id: 'valid-older', createdAt: 2, name: 'Valid'),
        ]),
      ],
    );
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);

    expect(await cache.refresh(const ['agent']), isTrue);
    expect(cache.state['agent'], isNull);
    expect(await cache.refresh(const ['agent']), isTrue);
    expect(cache.state['agent']?.displayName, 'Valid');
  });

  for (final preload in [false, true]) {
    test(
      '${preload ? 'preload' : 'refresh'} merges after parsing against newer live state',
      () async {
        final started = Completer<void>();
        final release = Completer<void>();
        final session = _RecordingProfileSession(
          result: Future.value([
            _profileEvent(id: 'old', createdAt: 1, name: 'Old'),
          ]),
        );
        final container = ProviderContainer(
          overrides: [
            relaySessionProvider.overrideWith(() => session),
            profileEventBatchParserProvider.overrideWithValue((events) async {
              final parsed = await parseProfileEventBatch(events);
              started.complete();
              await release.future;
              return parsed;
            }),
          ],
        );
        addTearDown(container.dispose);
        final cache = container.read(userCacheProvider.notifier);
        final result = preload
            ? cache.preload(['agent'])
            : cache.refresh(['agent']);
        await started.future;
        cache.cacheProfileEvent(
          _profileEvent(id: 'new', createdAt: 2, name: 'Live'),
        );
        cache.cacheProfileEvent(
          _profileEvent(
            id: 'other',
            pubkey: 'other',
            createdAt: 1,
            name: 'Other',
          ),
        );
        release.complete();
        expect(await result, isTrue);
        expect(cache.state['agent']?.displayName, 'Live');
        expect(cache.state['other']?.displayName, 'Other');
      },
    );

    test(
      '${preload ? 'preload' : 'refresh'} rejects a retired parsing completion',
      () async {
        final started = Completer<void>();
        final release = Completer<void>();
        final session = _RecordingProfileSession(
          result: Future.value([
            _profileEvent(id: 'old', createdAt: 1, name: 'Previous community'),
          ]),
        );
        final container = ProviderContainer(
          overrides: [
            relaySessionProvider.overrideWith(() => session),
            profileEventBatchParserProvider.overrideWithValue((events) async {
              final parsed = await parseProfileEventBatch(events);
              started.complete();
              await release.future;
              return parsed;
            }),
          ],
        );
        addTearDown(container.dispose);
        final cache = container.read(userCacheProvider.notifier);
        final result = preload
            ? cache.preload(['agent'])
            : cache.refresh(['agent']);
        await started.future;
        // This is the same dependency invalidation caused by community config.
        container
            .read(relayConfigProvider.notifier)
            .update(baseUrl: 'https://next-community.invalid');
        container.read(userCacheProvider);
        release.complete();
        expect(await result, isFalse);
        expect(container.read(userCacheProvider), isEmpty);
      },
    );
  }

  test(
    'community change rejects an old network response before parsing',
    () async {
      final response = Completer<List<NostrEvent>>();
      var parsed = false;
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(
            () => _RecordingProfileSession(result: response.future),
          ),
          profileEventBatchParserProvider.overrideWithValue((events) async {
            parsed = true;
            return parseProfileEventBatch(events);
          }),
        ],
      );
      addTearDown(container.dispose);
      final result = container.read(userCacheProvider.notifier).refresh([
        'agent',
      ]);
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://next-community.invalid');
      container.read(userCacheProvider);
      response.complete([_profileEvent(id: 'old', createdAt: 1, name: 'Old')]);
      expect(await result, isFalse);
      expect(parsed, isFalse);
      expect(container.read(userCacheProvider), isEmpty);
    },
  );

  test(
    'parser failure reports refresh failure without modifying cache',
    () async {
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(
            () => _RecordingProfileSession(
              result: Future.value([
                _profileEvent(id: 'next', createdAt: 2, name: 'Next'),
              ]),
            ),
          ),
          profileEventBatchParserProvider.overrideWithValue(
            (_) => Future.error(StateError('parse failed')),
          ),
        ],
      );
      addTearDown(container.dispose);
      final cache = container.read(userCacheProvider.notifier);
      cache.cacheProfileEvent(
        _profileEvent(id: 'cached', createdAt: 1, name: 'Cached'),
      );
      expect(await cache.refresh(['agent']), isFalse);
      expect(cache.state['agent']?.displayName, 'Cached');
    },
  );

  test('later refresh completion cannot roll back a newer refresh', () async {
    final oldResponse = Completer<List<NostrEvent>>();
    final session = _RecordingProfileSession(
      results: [
        oldResponse.future,
        Future.value([_profileEvent(id: 'new', createdAt: 2, name: 'New')]),
      ],
    );
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    final old = cache.refresh(['agent']);
    expect(await cache.refresh(['agent']), isTrue);
    oldResponse.complete([_profileEvent(id: 'old', createdAt: 1, name: 'Old')]);
    expect(await old, isTrue);
    expect(cache.state['agent']?.displayName, 'New');
  });

  test('stale malformed profile is skipped before parsing', () async {
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(
          () => _RecordingProfileSession(
            result: Future.value([
              NostrEvent(
                id: 'old',
                pubkey: 'agent',
                createdAt: 1,
                kind: 0,
                tags: [],
                content: '{"name":42}',
                sig: 'sig',
              ),
            ]),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    cache.cacheProfileEvent(
      _profileEvent(id: 'new', createdAt: 2, name: 'New'),
    );
    expect(await cache.refresh(['agent']), isTrue);
    expect(cache.state['agent']?.displayName, 'New');
  });

  test(
    'refresh and preload share one verification worker and coalesce pending lookups',
    () async {
      final started = Completer<void>();
      final release = Completer<void>();
      var running = 0;
      var maxRunning = 0;
      var calls = 0;
      final session = _RecordingProfileSession(
        results: [
          Future.value([
            _profileEvent(id: 'a', pubkey: 'a', createdAt: 1, name: 'A'),
          ]),
          Future.value([
            _profileEvent(id: 'b', pubkey: 'b', createdAt: 1, name: 'B'),
          ]),
          Future.value([
            _profileEvent(id: 'c', pubkey: 'c', createdAt: 1, name: 'C'),
          ]),
        ],
      );
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          profileEventBatchParserProvider.overrideWithValue((events) async {
            running++;
            if (running > maxRunning) maxRunning = running;
            if (calls++ == 0) {
              started.complete();
              await release.future;
            }
            final parsed = await parseProfileEventBatch(events);
            running--;
            return parsed;
          }),
        ],
      );
      addTearDown(container.dispose);
      final cache = container.read(userCacheProvider.notifier);
      final first = cache.preload(['a']);
      await started.future;
      final refresh = cache.refresh(['b']);
      final next = cache.preload(['c']);
      final duplicate = cache.preload(['c']);
      // Let the refresh reach the worker queue before releasing its predecessor.
      await Future<void>.delayed(Duration.zero);
      expect(calls, 1);
      release.complete();
      expect(
        await Future.wait([first, refresh, next, duplicate]),
        everyElement(isTrue),
      );
      expect(maxRunning, 1);
      expect(calls, 3);
      expect(session.requestCount, 3);
    },
  );

  test(
    'refresh verifies real OA batch while allowing main-isolate timers',
    () async {
      final owner = nostr.Keys.generate();
      final agents = List.generate(16, (_) => nostr.Keys.generate());
      final events = [
        for (final agent in agents)
          _profileEvent(
            id: agent.public,
            pubkey: agent.public,
            createdAt: 1,
            name: 'Synthetic',
            tags: [_authTag(owner, agent.public)],
          ),
      ];
      var timerRanBeforeParsingCompleted = false;
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(
            () => _RecordingProfileSession(result: Future.value(events)),
          ),
          profileEventBatchParserProvider.overrideWithValue((events) async {
            var timerRan = false;
            final timer = Timer(Duration.zero, () => timerRan = true);
            final parsed = await parseProfileEventBatch(events);
            timerRanBeforeParsingCompleted = timerRan;
            timer.cancel();
            return parsed;
          }),
        ],
      );
      addTearDown(container.dispose);
      final cache = container.read(userCacheProvider.notifier);
      expect(await cache.refresh(agents.map((a) => a.public).toList()), isTrue);
      expect(timerRanBeforeParsingCompleted, isTrue);
      for (final agent in agents) {
        expect(cache.state[agent.public]?.ownerPubkey, owner.public);
      }
    },
  );

  test('same-second profile tie keeps the lowest event id', () {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);

    cache.cacheProfileEvent(
      _profileEvent(id: 'b', createdAt: 1, name: 'Larger ID'),
    );
    cache.cacheProfileEvent(
      _profileEvent(id: 'a', createdAt: 1, name: 'Lower ID'),
    );
    cache.cacheProfileEvent(
      _profileEvent(id: 'c', createdAt: 1, name: 'Later Larger ID'),
    );

    expect(cache.state['agent']?.displayName, 'Lower ID');
  });
}

NostrEvent _profileEvent({
  required String id,
  required int createdAt,
  required String name,
  String pubkey = 'agent',
  List<List<String>> tags = const [],
  int kind = 0,
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: kind,
  tags: tags,
  content: jsonEncode({'name': name}),
  sig: 'sig',
);

List<String> _authTag(nostr.Keys owner, String agentPubkey) {
  final digest = SHA256Digest().process(
    Uint8List.fromList(
      utf8.encode('nostr:agent-auth:${agentPubkey.toLowerCase()}:'),
    ),
  );
  final message = digest
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  final signature = nostr.Schnorr.sign(
    secretKey: owner.secret,
    message: message,
  );
  return ['auth', owner.public, '', signature];
}

class _RecordingProfileSession extends RelaySessionNotifier {
  _RecordingProfileSession({
    Future<List<NostrEvent>>? result,
    List<Future<List<NostrEvent>>>? results,
  }) : _results = [...?results, ?result];

  final List<Future<List<NostrEvent>>> _results;
  NostrFilter? requestedFilter;
  int requestCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    requestCount++;
    requestedFilter = filter;
    return _results.isEmpty ? const [] : _results.removeAt(0);
  }
}

class _FailingProfileSession extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) => Future.error('profile unavailable');
}
