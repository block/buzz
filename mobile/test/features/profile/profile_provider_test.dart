import 'dart:convert';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  group('mergeProfileMetadata', () {
    test('preserves unexposed and unknown fields', () {
      final merged = mergeProfileMetadata(
        jsonEncode({
          'name': 'legacy-name',
          'display_name': 'Old',
          'picture': 'https://old.example/avatar.png',
          'about': 'Old bio',
          'nip05': 'alice@example.com',
          'custom': {'theme': 'bee'},
        }),
        displayName: '  Alice  ',
        avatarUrl: ' https://new.example/avatar.png ',
        about: ' Updated bio ',
      );

      expect(merged['display_name'], 'Alice');
      expect(merged['picture'], 'https://new.example/avatar.png');
      expect(merged['about'], 'Updated bio');
      expect(merged['name'], 'legacy-name');
      expect(merged['nip05'], 'alice@example.com');
      expect(merged['custom'], {'theme': 'bee'});
    });

    test('blank optional values remove only those fields', () {
      final merged = mergeProfileMetadata(
        '{"display_name":"Old","picture":"old","about":"old","nip05":"kept"}',
        displayName: 'New',
        avatarUrl: ' ',
        about: '',
      );

      expect(merged, {'display_name': 'New', 'nip05': 'kept'});
    });

    test('rejects an empty display name', () {
      expect(
        () => mergeProfileMetadata(
          null,
          displayName: ' ',
          avatarUrl: '',
          about: '',
        ),
        throwsArgumentError,
      );
    });
  });

  test(
    'updateProfile signs, publishes, preserves metadata, and updates state',
    () async {
      final keys = nostr.Keys.generate();
      final prior = _profileEvent(
        pubkey: keys.public,
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 10,
        content:
            '{"name":"legacy","display_name":"Old","nip05":"alice@example.com","custom":true}',
        tags: const [
          ['auth', 'owner'],
        ],
      );
      final relaySession = _RecordingRelaySession(prior: prior);
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(keys.nsec),
          ),
          relaySessionProvider.overrideWith(() => relaySession),
        ],
      );
      addTearDown(container.dispose);

      expect(await container.read(profileProvider.future), isA<UserProfile>());
      final updated = await container
          .read(profileProvider.notifier)
          .updateProfile(
            displayName: 'Alice',
            avatarUrl: 'https://example.com/avatar.png',
            about: 'Builder',
          );

      final published = relaySession.published.single;
      final content = jsonDecode(published.content) as Map<String, dynamic>;
      expect(published.kind, EventKind.metadata);
      expect(published.pubkey, keys.public);
      expect(published.sig, isNotEmpty);
      expect(published.createdAt, prior.createdAt + 1);
      expect(published.tags, prior.tags);
      expect(content['display_name'], 'Alice');
      expect(content['picture'], 'https://example.com/avatar.png');
      expect(content['about'], 'Builder');
      expect(content['name'], 'legacy');
      expect(content['nip05'], 'alice@example.com');
      expect(content['custom'], isTrue);
      expect(updated.displayName, 'Alice');
      expect(container.read(profileProvider).value?.displayName, 'Alice');
    },
  );

  test(
    'publish failure leaves the previously loaded profile unchanged',
    () async {
      final keys = nostr.Keys.generate();
      final prior = _profileEvent(
        pubkey: keys.public,
        createdAt: 1,
        content: '{"display_name":"Before"}',
      );
      final relaySession = _RecordingRelaySession(
        prior: prior,
        publishError: Exception('relay rejected'),
      );
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier(keys.nsec),
          ),
          relaySessionProvider.overrideWith(() => relaySession),
        ],
      );
      addTearDown(container.dispose);

      expect(
        (await container.read(profileProvider.future))?.displayName,
        'Before',
      );
      await expectLater(
        container
            .read(profileProvider.notifier)
            .updateProfile(displayName: 'After', avatarUrl: '', about: ''),
        throwsException,
      );
      expect(container.read(profileProvider).value?.displayName, 'Before');
    },
  );

  test('identity switch before signing aborts the update', () async {
    final firstKeys = nostr.Keys.generate();
    final secondKeys = nostr.Keys.generate();
    late _FakeRelayConfigNotifier configNotifier;
    final prior = _profileEvent(
      pubkey: firstKeys.public,
      createdAt: 1,
      content: '{"display_name":"Before"}',
    );
    var fetchCount = 0;
    final relaySession = _RecordingRelaySession(
      prior: prior,
      onFetch: () {
        fetchCount += 1;
        if (fetchCount == 2) {
          configNotifier.update(
            baseUrl: 'https://other-relay.example',
            nsec: secondKeys.nsec,
          );
        }
      },
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(() {
          configNotifier = _FakeRelayConfigNotifier(firstKeys.nsec);
          return configNotifier;
        }),
        relaySessionProvider.overrideWith(() => relaySession),
      ],
    );
    addTearDown(container.dispose);

    expect(
      (await container.read(profileProvider.future))?.displayName,
      'Before',
    );
    await expectLater(
      container
          .read(profileProvider.notifier)
          .updateProfile(displayName: 'After', avatarUrl: '', about: ''),
      throwsStateError,
    );
    expect(relaySession.published, isEmpty);
  });
}

NostrEvent _profileEvent({
  required String pubkey,
  required int createdAt,
  required String content,
  List<List<String>> tags = const [],
}) => NostrEvent(
  id: 'prior',
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.metadata,
  tags: tags,
  content: content,
  sig: 'sig',
);

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  _FakeRelayConfigNotifier(this.nsec);

  final String nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _RecordingRelaySession extends RelaySessionNotifier {
  _RecordingRelaySession({
    required this.prior,
    this.publishError,
    this.onFetch,
  });

  final NostrEvent prior;
  final Object? publishError;
  final void Function()? onFetch;
  final List<NostrEvent> published = [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    onFetch?.call();
    return [prior];
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    if (publishError case final error?) throw error;
    return NostrEvent(
      id: event.id,
      pubkey: event.pubkey,
      createdAt: event.createdAt,
      kind: event.kind,
      tags: event.tags,
      content: 'saved',
      sig: event.sig,
    );
  }
}
