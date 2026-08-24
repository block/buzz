import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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
      const NostrEvent(
        id: 'cached-profile',
        pubkey: 'agent',
        createdAt: 1,
        kind: 0,
        tags: [],
        content: '{"name":"Cached Human"}',
        sig: 'sig',
      ),
    );

    final succeeded = await cache.refresh(const ['AGENT']);

    expect(succeeded, isTrue);
    expect(session.requestedFilter?.kinds, const [0]);
    expect(session.requestedFilter?.authors, const ['agent']);
    expect(session.requestedFilter?.limit, 1);
  });
}

class _RecordingProfileSession extends RelaySessionNotifier {
  NostrFilter? requestedFilter;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    requestedFilter = filter;
    return const [];
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
