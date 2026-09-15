import 'dart:async';

import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/crypto/nip_oa_test.dart' show profile;

void main() {
  for (final cached in [true, false]) {
    for (final result in ['signed', 'invalid', 'empty', 'malformed', 'error']) {
      testWidgets('about opening snapshot: cached=$cached, $result', (
        tester,
      ) async {
        final keys = nostr.Keys.generate();
        final initial = profile(
          keys,
          [],
          createdAt: 1,
          content: '{"name":"Cached name","about":"Cached about"}',
        );
        final fresh = profile(
          keys,
          [],
          createdAt: 2,
          content:
              '{"name":"Fresh name","about":"Fresh about",'
              '"picture":"https://fresh.example/avatar.png"}',
        );
        final invalid = NostrEvent.fromJson({
          ...fresh.toJson(),
          'id': '0' * 64,
          'created_at': 3,
          'content': '{"about":"Forged about"}',
        });
        final session = _SheetSession();
        final container = ProviderContainer.test(
          overrides: [
            relayConfigProvider.overrideWith(_SheetConfig.new),
            relaySessionProvider.overrideWith(() => session),
            activeCommunityProvider.overrideWith((ref) async => null),
            currentPubkeyProvider.overrideWithValue(keys.public),
            presenceCacheProvider.overrideWith(_SheetPresence.new),
            userStatusCacheProvider.overrideWith(_SheetStatus.new),
          ],
        );
        final cache = container.read(userCacheProvider.notifier);
        if (cached) cache.captureAdmission().add(initial);
        await tester.pumpWidget(
          UncontrolledProviderScope(
            container: container,
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(body: UserProfileSheet(pubkey: keys.public)),
            ),
          ),
        );
        expect(session.requests, hasLength(1));
        expect(
          find.text('Cached about'),
          cached ? findsOneWidget : findsNothing,
        );
        await tester.pump(const Duration(milliseconds: 60));
        // Existing preload/get coalescing: no new refresh of a cached identity.
        expect(session.requests, hasLength(cached ? 1 : 2));
        if (!cached) {
          session.requests[1].complete([initial]);
          await tester.pump();
          await tester.pump();
          expect(find.text('Cached name'), findsOneWidget);
          expect(find.text('Cached about'), findsOneWidget);
        }
        if (result == 'error') {
          session.requests[0].completeError(StateError('relay failed'));
        } else {
          session.requests[0].complete(switch (result) {
            'signed' => [invalid, initial, fresh],
            'invalid' => [invalid],
            'malformed' => [profile(keys, [], content: 'not-json')],
            _ => <NostrEvent>[],
          });
        }
        await tester.pump();
        await tester.pump();
        expect(find.text('Forged about'), findsNothing);
        expect(find.text('Cached about'), findsNothing);
        expect(
          find.text('Fresh about'),
          result == 'signed' ? findsOneWidget : findsNothing,
        );
        expect(find.text('Cached name'), findsOneWidget);
        expect(find.text('Fresh name'), findsNothing);
        expect(cache.state[keys.public]?.avatarUrl, isNull);
        cache.captureAdmission().add(
          profile(
            keys,
            [],
            createdAt: 4,
            content: '{"name":"Live name","about":"Live about"}',
          ),
        );
        await tester.pump();
        expect(find.text('Live name'), findsOneWidget);
        expect(find.text('Live about'), findsNothing);
        expect(session.requests, hasLength(cached ? 1 : 2));

        // Same identity, different community: don't retain a completed snapshot.
        final config = container.read(relayConfigProvider.notifier);
        config.update(baseUrl: 'https://b.example', nsec: null);
        await tester.pump();
        expect(find.text('Fresh about'), findsNothing);
        expect(find.text('Live about'), findsNothing);
        expect(find.text('Live name'), findsNothing);
        final b = session.requests.last;
        final beforeC = session.requests.length;
        config.update(baseUrl: 'https://c.example', nsec: null);
        await tester.pump();
        expect(session.requests, hasLength(beforeC + 1));
        final c = session.requests.last;
        b.complete([profile(keys, [], content: '{"about":"Retired B"}')]);
        await tester.pump();
        expect(find.text('Retired B'), findsNothing);
        expect(container.read(userCacheProvider), isEmpty);
        c.complete([profile(keys, [], content: '{"about":"Current C"}')]);
        await tester.pump();
        await tester.pump();
        expect(find.text('Current C'), findsOneWidget);
        // Resetting the cache retires the captured generation even at the same URL.
        final beforeReset = session.requests.length;
        container.invalidate(userCacheProvider);
        await tester.pump();
        expect(find.text('Current C'), findsNothing);
        expect(session.requests, hasLength(beforeReset + 1));
        session.requests.last.complete([]);
        await tester.pump();
        expect(find.text('Current C'), findsNothing);
        await tester.pumpWidget(const SizedBox.shrink());
        container.dispose();
      });
    }
  }
}

class _SheetConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() => RelayConfig(baseUrl: 'https://a.example', nsec: null);
  @override
  void update({required String baseUrl, String? nsec}) {
    state = RelayConfig(baseUrl: baseUrl, nsec: nsec);
  }
}

class _SheetSession extends RelaySessionNotifier {
  final requests = <Completer<List<NostrEvent>>>[];
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    expect(filter.kinds, [0]);
    final request = Completer<List<NostrEvent>>();
    requests.add(request);
    return request.future;
  }
}

class _SheetPresence extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {};
  @override
  void track(List<String> pubkeys) {}
}

class _SheetStatus extends UserStatusCacheNotifier {
  @override
  Map<String, UserStatus?> build() => {};
  @override
  void track(List<String> pubkeys) {}
}
