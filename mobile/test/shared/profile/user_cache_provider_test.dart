import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
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
    final agent = nostr.Keys.generate();
    final session = _RecordingProfileSession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    cache.captureAdmission().add(
      _profileEvent(keys: agent, createdAt: 1, name: 'Cached Human'),
    );

    final succeeded = await cache.refresh([agent.public.toUpperCase()]);

    expect(succeeded, isTrue);
    expect(session.requestedFilter?.kinds, const [0]);
    expect(session.requestedFilter?.authors, [agent.public]);
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

    cache.captureAdmission().add(
      _profileEvent(
        keys: agent,
        createdAt: 2,
        name: 'Agent',
        tags: [_authTag(owner, agent.public)],
      ),
    );
    refreshCompleter.complete([
      _profileEvent(keys: agent, createdAt: 1, name: 'Human'),
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
        _profileEvent(keys: agent, createdAt: 2, name: 'Human'),
      ]),
    );
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);
    cache.captureAdmission().add(
      _profileEvent(
        keys: agent,
        createdAt: 1,
        name: 'Agent',
        tags: [_authTag(owner, agent.public)],
      ),
    );

    expect(await cache.refresh([agent.public]), isTrue);
    cache.put(UserProfile(pubkey: agent.public, ownerPubkey: owner.public));
    expect(cache.state[agent.public]?.displayName, 'Human');
    expect(cache.state[agent.public]?.ownerPubkey, isNull);
  });

  test('non-profile history cannot poison profile order', () async {
    final agent = nostr.Keys.generate();
    final session = _RecordingProfileSession(
      results: [
        Future.value([
          _profileEvent(keys: agent, createdAt: 3, name: 'Ignored', kind: 1),
        ]),
        Future.value([_profileEvent(keys: agent, createdAt: 2, name: 'Valid')]),
      ],
    );
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);

    expect(await cache.refresh([agent.public]), isTrue);
    expect(cache.state[agent.public], isNull);
    expect(await cache.refresh([agent.public]), isTrue);
    expect(cache.state[agent.public]?.displayName, 'Valid');
  });

  test('same-second profile tie keeps the lowest event id', () {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final cache = container.read(userCacheProvider.notifier);

    final agent = nostr.Keys.generate();
    final events = [
      for (final name in ['First', 'Second', 'Third'])
        _profileEvent(keys: agent, createdAt: 1, name: name),
    ]..sort((a, b) => a.id.compareTo(b.id));
    for (final event in [events[1], events[0], events[2]]) {
      cache.captureAdmission().add(event);
    }

    expect(
      cache.state[agent.public]?.displayName,
      ProfileData.fromEvent(events.first).displayName,
    );
  });
}

NostrEvent _profileEvent({
  required nostr.Keys keys,
  required int createdAt,
  required String name,
  List<List<String>> tags = const [],
  int kind = 0,
}) => NostrEvent.fromJson(
  nostr.Event.from(
    secretKey: keys.secret,
    createdAt: createdAt,
    kind: kind,
    tags: tags,
    content: jsonEncode({'name': name}),
  ).toMap(),
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
