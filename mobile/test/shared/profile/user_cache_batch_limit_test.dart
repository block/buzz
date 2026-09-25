import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const bridge = MethodChannel('buzz/push');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

  test(
    'pending profiles drain in bounded queries after delayed export',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final entered = Completer<void>();
      final release = Completer<void>();
      messenger.setMockMethodCallHandler(bridge, (_) async {
        if (!entered.isCompleted) {
          entered.complete();
          await release.future;
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(bridge, null);
        debugDefaultTargetPlatformOverride = null;
      });
      final signed = NostrEvent.fromJson(
        nostr.Event.from(
          kind: 0,
          createdAt: 1,
          content: '{"name":"First"}',
          tags: [],
          secretKey: '1'.padLeft(64, '0'),
        ).toMap(),
      );
      final session = _ClampingSession(first: signed);
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          activeCommunityProvider.overrideWith(
            (ref) async => Community(
              id: 'original',
              name: 'Synthetic',
              relayUrl: 'https://example.invalid',
              addedAt: DateTime(2026),
            ),
          ),
        ],
      );
      addTearDown(container.dispose);
      await container.read(activeCommunityProvider.future);
      final cache = container.read(userCacheProvider.notifier);
      expect(await cache.preload([signed.pubkey]), isTrue);
      await entered.future.timeout(const Duration(seconds: 5));
      final keys = List.generate(1001, (i) => 'profile-$i');
      var completed = false;
      final pending = cache.preload(keys).then((result) {
        completed = true;
        return result;
      });
      final duplicate = cache.preload(keys);
      try {
        await Future<void>.delayed(const Duration(milliseconds: 60));
        expect(session.filters, hasLength(1));
        expect(completed, isFalse);
      } finally {
        release.complete();
      }
      expect(await pending, isTrue);
      expect(await duplicate, isTrue);
      expect(container.read(userCacheProvider).keys, containsAll(keys));
      expect(session.filters.map((f) => f.authors!.length), [1, 1000, 1]);
      expect(session.filters.map((f) => f.limit), [1, 1000, 1]);
      expect(session.maxRunning, 1);
    },
  );

  for (final preload in [true, false]) {
    for (final outcome in ['success', 'failure', 'community change']) {
      test(
        '${preload ? "preload" : "refresh"} waits for final bounded query: $outcome',
        () async {
          final started = Completer<void>();
          final release = Completer<void>();
          final session = _ClampingSession(
            beforeFetch: (index) async {
              if (index != 1) return;
              started.complete();
              await release.future;
              if (outcome == 'failure') throw StateError('relay unavailable');
            },
          );
          final container = ProviderContainer(
            overrides: [relaySessionProvider.overrideWith(() => session)],
          );
          addTearDown(container.dispose);
          final cache = container.read(userCacheProvider.notifier);
          final keys = List.generate(1001, (i) => 'profile-$i');
          bool? result;
          final done = (preload ? cache.preload(keys) : cache.refresh(keys))
              .then((value) => result = value);
          await started.future.timeout(const Duration(seconds: 5));
          expect(result, isNull);
          expect(container.read(userCacheProvider), hasLength(1000));
          if (outcome == 'community change') {
            container
                .read(relayConfigProvider.notifier)
                .update(baseUrl: 'https://next.invalid');
            container.read(userCacheProvider);
          }
          release.complete();
          await done;
          expect(result, outcome == 'success');
          expect(session.filters.map((f) => f.authors!.length), [1000, 1]);
          expect(session.maxRunning, 1);
          expect(
            container.read(userCacheProvider),
            hasLength(
              outcome == 'community change'
                  ? 0
                  : outcome == 'failure'
                  ? 1000
                  : 1001,
            ),
          );
        },
      );
    }
  }
}

class _ClampingSession extends RelaySessionNotifier {
  _ClampingSession({this.first, this.beforeFetch});
  final NostrEvent? first;
  final Future<void> Function(int)? beforeFetch;
  final filters = <NostrFilter>[];
  int running = 0;
  int maxRunning = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final index = filters.length;
    filters.add(filter);
    running++;
    if (running > maxRunning) maxRunning = running;
    try {
      await beforeFetch?.call(index);
      return [
        for (final key in filter.authors!.take(1000))
          if (key == first?.pubkey)
            first!
          else
            NostrEvent(
              id: key,
              pubkey: key,
              createdAt: 1,
              kind: 0,
              tags: [],
              content: jsonEncode({'name': key}),
              sig: 'synthetic',
            ),
      ];
    } finally {
      running--;
    }
  }
}
