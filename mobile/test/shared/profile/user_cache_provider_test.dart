import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/shared/profile/user_cache_provider.dart';
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

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
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
