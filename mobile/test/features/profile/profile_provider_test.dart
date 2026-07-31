import 'dart:convert';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('signs kind:0, preserves metadata, and refreshes both caches', () async {
    final keys = nostr.Keys.generate();
    final previous = _profileEvent(
      keys,
      content: {
        'name': 'old-handle',
        'display_name': 'Old Name',
        'picture': 'https://example.com/old.png',
        'about': 'Still here',
        'nip05': 'old@example.com',
        'custom': {'keep': true},
      },
      tags: const [
        ['x', 'keep'],
      ],
    );
    final session = _FakeRelaySession(events: [previous]);
    final container = _container(session: session, nsec: keys.nsec);
    addTearDown(container.dispose);
    await container.read(profileProvider.future);

    const avatar =
        'data:image/svg+xml,%3Csvg%3E%3Ctext%3E%F0%9F%90%9D%3C%2Ftext%3E%3C%2Fsvg%3E';
    final saved = await container
        .read(profileProvider.notifier)
        .saveProfile(displayName: '  New Name  ', avatarUrl: avatar);

    final published = session.published!;
    final verified = nostr.Event.fromJson(jsonEncode(published.toJson()));
    final metadata = jsonDecode(published.content) as Map<String, dynamic>;
    expect(verified.isValid(), isTrue);
    expect(published.kind, EventKind.metadata);
    expect(published.pubkey, keys.public);
    expect(published.tags, previous.tags);
    expect(published.createdAt, greaterThan(previous.createdAt));
    expect(metadata['display_name'], 'New Name');
    expect(metadata['picture'], avatar);
    expect(metadata['name'], 'old-handle');
    expect(metadata['about'], 'Still here');
    expect(metadata['nip05'], 'old@example.com');
    expect(metadata['custom'], {'keep': true});

    expect(saved.displayName, 'New Name');
    expect(saved.avatarUrl, avatar);
    expect(container.read(profileProvider).value, saved);
    expect(container.read(userCacheProvider)[keys.public], saved);
  });

  test('keeps the loaded profile when relay publication fails', () async {
    final keys = nostr.Keys.generate();
    final previous = _profileEvent(
      keys,
      content: {'display_name': 'Original', 'about': 'Keep me'},
    );
    final session = _FakeRelaySession(
      events: [previous],
      publishError: Exception('relay rejected profile'),
    );
    final container = _container(session: session, nsec: keys.nsec);
    addTearDown(container.dispose);
    final original = await container.read(profileProvider.future);

    await expectLater(
      container
          .read(profileProvider.notifier)
          .saveProfile(displayName: 'Unsaved'),
      throwsException,
    );

    expect(container.read(profileProvider).value, same(original));
    expect(container.read(userCacheProvider), isEmpty);
  });

  test('fails closed without an active community signing key', () async {
    final session = _FakeRelaySession();
    final container = _container(session: session, nsec: null);
    addTearDown(container.dispose);
    await container.read(profileProvider.future);

    await expectLater(
      container
          .read(profileProvider.notifier)
          .saveProfile(displayName: 'No Key'),
      throwsA(isA<StateError>()),
    );

    expect(session.published, isNull);
    expect(session.fetchCount, 0);
  });
}

ProviderContainer _container({
  required _FakeRelaySession session,
  required String? nsec,
}) {
  return ProviderContainer(
    overrides: [
      relaySessionProvider.overrideWith(() => session),
      relayConfigProvider.overrideWith(
        () => _FakeRelayConfigNotifier(nsec: nsec),
      ),
    ],
  );
}

NostrEvent _profileEvent(
  nostr.Keys keys, {
  required Map<String, dynamic> content,
  List<List<String>> tags = const [],
}) {
  final event = nostr.Event.from(
    kind: EventKind.metadata,
    content: jsonEncode(content),
    tags: tags,
    secretKey: nostr.Nip19.decode(payload: keys.nsec).data,
    createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 10,
    verify: false,
  );
  return NostrEvent.fromJson(event.toMap());
}

class _FakeRelaySession extends RelaySessionNotifier {
  _FakeRelaySession({this.events = const [], this.publishError});

  final List<NostrEvent> events;
  final Object? publishError;
  NostrEvent? published;
  int fetchCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    fetchCount++;
    return events;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published = event;
    if (publishError case final error?) throw error;
    return event;
  }
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  _FakeRelayConfigNotifier({required this.nsec});

  final String? nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}
