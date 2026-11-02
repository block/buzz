import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

import '../../../shared/me/me_sync_test.dart' show FakeRelay;

const _id = '11111111-1111-4111-8111-111111111111';

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: _id,
      name: 'Saved chat',
      channelType: 'stream',
      visibility: 'private',
      description: '',
      createdBy: 'owner',
      createdAt: DateTime(2026),
      memberCount: 2,
      isMember: true,
    ),
  ];
}

class _Store extends InMemorySharedPreferencesStore {
  bool fail = false;
  Completer<bool>? blocked;
  _Store() : super.empty();
  @override
  Future<bool> setValue(String type, String key, Object value) async {
    if (key.startsWith('flutter.me-cache:')) {
      if (fail) return false;
      if (blocked != null && !await blocked!.future) return false;
    }
    return super.setValue(type, key, value);
  }
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _Navigation extends NavigatorObserver {
  int pushes = 0;
  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pushes++;
  }
}

void main() {
  testWidgets(
    'offline-save warning keeps Solo rows usable until retry succeeds',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final store = _Store();
      SharedPreferencesStorePlatform.instance = store;
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      const secret =
          '0000000000000000000000000000000000000000000000000000000000000001';
      final service = MeSync(
        relay,
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: secret,
          ),
        ),
        prefs: prefs,
      );
      NostrEvent head(int time) => NostrEvent.fromJson(
        nostr.Event.from(
          kind: 30078,
          createdAt: time,
          secretKey: secret,
          content: nip44Encrypt(
            service.key,
            const MeGroups(channels: [_id]).encode(service.community),
          ),
          tags: [
            ['d', MeGroups.coordinate(service.community)],
            ['t', MeGroups.tag],
          ],
        ).toMap(),
      );
      final old = head(1), fresh = head(2);
      await service.cache(old);
      relay.events = [fresh];
      store.fail = true;
      final navigation = _Navigation();
      final navigator = GlobalKey<NavigatorState>();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            meSyncProvider.overrideWithValue(service),
            meAssistantsProvider.overrideWith((ref) async => []),
            channelsProvider.overrideWith(_Channels.new),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            navigatorKey: navigator,
            navigatorObservers: [navigation],
            home: const MePage(),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Saved chat'), findsOneWidget);
      expect(
        find.text(
          'Your chats are available, but couldn’t be saved for offline use.',
        ),
        findsOneWidget,
      );
      expect(find.text('Couldn’t load your chats and projects.'), findsNothing);
      expect(service.cachedSnapshot()?.event?.id, old.id);
      await tester.tap(find.text('Saved chat'));
      await tester.pump();
      expect(navigation.pushes, 2);
      navigator.currentState!.pop();
      await tester.pumpAndSettle();
      store.fail = false;
      store.blocked = Completer<bool>();
      await tester.tap(find.text('Retry offline save'));
      await tester.pumpAndSettle();
      expect(find.text('Saved chat'), findsOneWidget);
      expect(find.text('Retry offline save'), findsOneWidget);
      store.blocked!.complete(true);
      await tester.pumpAndSettle();
      expect(find.text('Retry offline save'), findsNothing);
      expect(find.text('Saved chat'), findsOneWidget);
      expect(service.cachedSnapshot()?.event?.id, fresh.id);

      // A closed reader resubscribes on resume without user action.
      final scope = ProviderScope.containerOf(
        tester.element(find.byType(MePage)),
      );
      final lifecycle = scope.read(appLifecycleProvider.notifier);
      final rejected = relay.closeSubscription;
      relay.closeSubscription!('terminal CLOSED');
      lifecycle.state = AppLifecycleState.paused;
      lifecycle.state = AppLifecycleState.resumed;
      await tester.pumpAndSettle();
      expect(relay.closeSubscription, isNot(same(rejected)));
      final newer = head(3);
      relay.emit!(newer);
      await tester.pumpAndSettle();
      expect(service.cachedSnapshot()?.event?.id, newer.id);
      expect(find.text('Saved chat'), findsOneWidget);
      expect(find.textContaining('Live updates'), findsNothing);

      final invalid = NostrEvent.fromJson({
        ...head(4).toJson(),
        'sig': '0' * 128,
      });
      relay.emit!(invalid);
      await tester.pumpAndSettle();
      expect(find.text('Saved chat'), findsOneWidget);
      expect(service.cachedSnapshot()?.event?.id, newer.id);
    },
  );
}
