import 'dart:async';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/me/me_recovery.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';

class _Config extends RelayConfigNotifier {
  final RelayConfig config;
  _Config(this.config);
  @override
  RelayConfig build() => config;
  void switchTo(RelayConfig next) => state = next;
}

class FakeRelay extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  void setStatus(SessionStatus status) => state = SessionState(status: status);
  int directoryQueries = 0;
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    directoryQueries++;
    return [];
  }

  List<NostrEvent> events = [];
  Future<List<NostrEvent>>? pendingRead;
  void Function(String)? closeSubscription;
  void Function(NostrEvent)? emit;
  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    closeSubscription = onClosed;
    emit = onEvent;
    return () {};
  }

  int writes = 0;
  bool loseAcknowledgment = false;
  void Function()? beforeRead;
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    beforeRead?.call();
    return pendingRead ?? events;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    writes++;
    events = [event];
    if (loseAcknowledgment) throw TimeoutException('no acknowledgment');
    return event;
  }
}

class _FailingCacheStore extends InMemorySharedPreferencesStore {
  String? failure;
  String? removeFailure;
  Completer<bool>? blockedWrite;
  int cacheWrites = 0;
  _FailingCacheStore() : super.empty();
  @override
  Future<bool> setValue(String type, String key, Object value) {
    if (key.startsWith('flutter.me-cache:')) {
      cacheWrites++;
      if (blockedWrite != null) return blockedWrite!.future;
      if (failure == 'throw') throw StateError('disk error');
      if (failure == 'false') return Future.value(false);
    }
    return super.setValue(type, key, value);
  }

  @override
  Future<bool> remove(String key) async {
    if (key.startsWith('flutter.me-pending:')) {
      if (removeFailure == 'false') return false;
      if (removeFailure == 'throw') throw StateError('remove failed');
    }
    return super.remove(key);
  }
}

void main() {
  const secret =
      '0000000000000000000000000000000000000000000000000000000000000001';
  final nsec = nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: secret);
  const origin = 'https://relay.test';
  final public = nostr.Keys(secret).public;
  NostrEvent signed({
    String community = origin,
    String signingKey = secret,
    int timestamp = 1,
    MeGroups groups = const MeGroups(),
  }) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: 30078,
      content: nip44Encrypt(
        getConversationKey(secret, public),
        groups.encode(community),
      ),
      secretKey: signingKey,
      createdAt: timestamp,
      tags: [
        ['d', MeGroups.coordinate(community)],
        ['t', MeGroups.tag],
      ],
      verify: false,
    ).toMap(),
  );
  for (final failure in ['false', 'throw']) {
    test(
      'chat recovery reconciles its confirmed Solo journal after $failure cleanup',
      () async {
        SharedPreferences.setMockInitialValues({});
        final store = _FailingCacheStore()..removeFailure = failure;
        SharedPreferencesStorePlatform.instance = store;
        final prefs = await SharedPreferences.getInstance();
        final relay = FakeRelay();
        final sync = MeSync(
          relay,
          RelayConfig(baseUrl: origin, nsec: nsec),
          prefs: prefs,
        );
        const id = '11111111-1111-4111-8111-111111111111';
        const intent = MeChatCreation(id, 'Chat', 'agent');
        Future<String> complete() => intent.complete(
          sync,
          findChannel: () async => id,
          createChannel: () async => id,
          hasAgent: () async => true,
          addAgent: () async {},
          isPlaced: () async =>
              relay.events.isNotEmpty &&
              sync.decode(relay.events.single).channels.contains(id),
          place: () => sync.update((current) => current.place(id)),
        );
        await expectLater(complete(), throwsStateError);
        expect(prefs.getString(sync.pendingKey), isNotNull);
        expect(MeChatCreation.pending(sync), isNotNull);
        store.removeFailure = null;
        await expectLater(complete(), throwsStateError);
        expect(prefs.getString(sync.pendingKey), isNull);
        expect(MeChatCreation.pending(sync), isNotNull);
        expect(await complete(), id);
        expect(MeChatCreation.pending(sync), isNull);
        expect(relay.writes, 1);
      },
    );
  }
  for (final failure in ['false', 'throw']) {
    test(
      'pending-save discard retains recovery after $failure removal',
      () async {
        SharedPreferences.setMockInitialValues({});
        final store = _FailingCacheStore();
        SharedPreferencesStorePlatform.instance = store;
        final prefs = await SharedPreferences.getInstance();
        final service = MeSync(
          FakeRelay(),
          RelayConfig(baseUrl: origin, nsec: nsec),
          prefs: prefs,
        );
        await prefs.setString(service.pendingKey, 'reviewed');
        store.removeFailure = failure;
        await expectLater(service.discardPending('reviewed'), throwsStateError);
        expect(prefs.getString(service.pendingKey), 'reviewed');
        store.removeFailure = null;
        await service.discardPending('reviewed');
        expect(prefs.getString(service.pendingKey), isNull);
      },
    );
  }
  test(
    'dirty newest snapshot survives transport reconnect and remains scoped',
    () async {
      SharedPreferences.setMockInitialValues({});
      final store = _FailingCacheStore();
      SharedPreferencesStorePlatform.instance = store;
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      final config = _Config(RelayConfig(baseUrl: origin, nsec: nsec));
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(() => config),
          relaySessionProvider.overrideWith(() => relay),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
      );
      addTearDown(container.dispose);
      final service = container.read(meSyncProvider);
      container.listen(meAssistantsProvider, (_, _) {});
      await container.read(meAssistantsProvider.future);
      expect(relay.directoryQueries, 1);
      final a = signed(timestamp: 1);
      final b = signed(
        timestamp: 2,
        groups: const MeGroups(
          groups: [
            {'id': 'new', 'name': 'Newest', 'defaultTemplateId': ''},
          ],
        ),
      );
      await service.cache(a);
      relay.events = [a];
      container.listen(meGroupsProvider, (_, _) {});
      container.listen(meOfflineSaveWarningProvider, (_, _) {});
      await container.read(meGroupsProvider.future);
      store.failure = 'false';
      relay.emit!(b);
      await Future<void>.delayed(Duration.zero);
      expect(
        container.read(meGroupsProvider).value!.groups.single['name'],
        'Newest',
      );
      expect(service.cacheWriteFailed.value, isTrue);
      relay.setStatus(SessionStatus.disconnected);
      await container.pump();
      expect(container.read(meSyncProvider), same(service));
      expect(service.connected, isFalse);
      expect(
        container.read(meGroupsProvider).value!.groups.single['name'],
        'Newest',
      );
      expect(container.read(meOfflineSaveWarningProvider).value, isTrue);
      relay.setStatus(SessionStatus.connected);
      await container.pump();
      await container.read(meGroupsProvider.future);
      await Future<void>.delayed(Duration.zero);
      expect(container.read(meSyncProvider), same(service));
      expect(service.connected, isTrue);
      await container.read(meAssistantsProvider.future);
      expect(relay.directoryQueries, 2);
      expect(
        container.read(meGroupsProvider).value!.groups.single['name'],
        'Newest',
      );
      expect(service.cacheWriteFailed.value, isTrue);
      store.failure = null;
      container.invalidate(meGroupsProvider);
      await container.read(meGroupsProvider.future);
      await Future<void>.delayed(Duration.zero);
      expect(service.cachedSnapshot()?.event?.id, b.id);
      expect(service.cacheWriteFailed.value, isFalse);
      config.switchTo(RelayConfig(baseUrl: 'https://other.test', nsec: nsec));
      await container.pump();
      final other = container.read(meSyncProvider);
      expect(service.disposed, isTrue);
      expect(other.latestCacheCandidate, isNull);
      expect(other.cacheWriteFailed.value, isFalse);
    },
  );
  test(
    'newer failed cache fences older arrivals and Retry retains newest',
    () async {
      SharedPreferences.setMockInitialValues({});
      final store = _FailingCacheStore();
      SharedPreferencesStorePlatform.instance = store;
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      final service = MeSync(
        relay,
        RelayConfig(baseUrl: origin, nsec: nsec),
        prefs: prefs,
      );
      final initial = signed(timestamp: 1);
      await service.cache(initial);
      relay.events = [initial];
      final container = ProviderContainer(
        overrides: [meSyncProvider.overrideWithValue(service)],
      );
      addTearDown(container.dispose);
      final seen = <String>[];
      container.listen(meGroupsProvider, (_, value) {
        if (value.hasValue) {
          seen.add(
            value.value!.groups.firstOrNull?['name'] as String? ?? 'initial',
          );
        }
      });
      container.listen(meOfflineSaveWarningProvider, (_, _) {});
      await container.read(meGroupsProvider.future);
      await Future<void>.delayed(Duration.zero);
      final newer = signed(
        timestamp: 3,
        groups: const MeGroups(
          groups: [
            {'id': 'new', 'name': 'New', 'defaultTemplateId': ''},
          ],
        ),
      );
      final older = signed(
        timestamp: 2,
        groups: const MeGroups(
          groups: [
            {'id': 'old', 'name': 'Old', 'defaultTemplateId': ''},
          ],
        ),
      );
      store.blockedWrite = Completer<bool>();
      relay.emit!(newer);
      await Future<void>.delayed(Duration.zero);
      final writes = store.cacheWrites;
      relay.emit!(older);
      // Also exercise the persistence fence independently of the reader.
      final olderWrite = service.cache(older);
      final blockedWrite = store.blockedWrite!;
      store.blockedWrite = null;
      blockedWrite.complete(false);
      await olderWrite;
      await Future<void>.delayed(Duration.zero);
      expect(store.cacheWrites, writes);
      expect(seen, isNot(contains('Old')));
      expect(
        container.read(meGroupsProvider).value!.groups.single['name'],
        'New',
      );
      expect(service.cacheWriteFailed.value, isTrue);
      expect(service.cachedSnapshot()?.event?.id, initial.id);
      store.blockedWrite = null;
      relay.events = [older];
      container.invalidate(meGroupsProvider);
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(service.cacheWriteFailed.value, isFalse);
      expect(service.cachedSnapshot()?.event?.id, newer.id);
    },
  );

  for (final absent in [false, true]) {
    test(
      'confirmed cleanup preserves ${absent ? 'already absent' : 'replaced'} journal',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final relay = FakeRelay();
        final service = MeSync(
          relay,
          RelayConfig(baseUrl: origin, nsec: nsec),
          prefs: prefs,
        );
        relay.beforeRead = () {
          if (relay.writes > 0) {
            if (absent) {
              unawaited(prefs.remove(service.pendingKey));
            } else {
              unawaited(
                prefs.setString(service.pendingKey, 'another pending record'),
              );
            }
          }
        };
        if (absent) {
          await service.update((v) => v);
          expect(prefs.getString(service.pendingKey), isNull);
        } else {
          await expectLater(service.update((v) => v), throwsStateError);
          expect(prefs.getString(service.pendingKey), 'another pending record');
        }
      },
    );
  }
  for (final recovered in [false, true]) {
    for (final failure in ['false', 'throw']) {
      test(
        'confirmed ${recovered ? 'recovery' : 'normal'} cleanup $failure remains retryable',
        () async {
          SharedPreferences.setMockInitialValues({});
          final store = _FailingCacheStore();
          SharedPreferencesStorePlatform.instance = store;
          final prefs = await SharedPreferences.getInstance();
          final relay = FakeRelay();
          final config = RelayConfig(baseUrl: origin, nsec: nsec);
          var service = MeSync(relay, config, prefs: prefs);
          if (recovered) {
            relay.loseAcknowledgment = true;
            await expectLater(
              service.update((v) => v),
              throwsA(isA<TimeoutException>()),
            );
            relay.loseAcknowledgment = false;
            service = MeSync(relay, config, prefs: prefs);
          }
          store.removeFailure = failure;
          await expectLater(
            service.update((v) => v),
            throwsA(
              isA<StateError>().having(
                (e) => e.message,
                'message',
                contains('cleanup on this phone failed'),
              ),
            ),
          );
          await prefs.reload();
          expect(prefs.getString(service.pendingKey), isNotNull);
          expect(relay.writes, 1);
          store.removeFailure = null;
          await expectLater(
            MeSync(relay, config, prefs: prefs).update((v) => v),
            throwsA(
              isA<StateError>().having(
                (e) => e.message,
                'message',
                contains('previous Solo save is confirmed'),
              ),
            ),
          );
          expect(prefs.getString(service.pendingKey), isNull);
          expect(relay.writes, 1);
        },
      );
    }
  }
  for (final failure in ['false', 'throw']) {
    test(
      'snapshot write $failure exposes failure and Retry persists the same event',
      () async {
        SharedPreferences.setMockInitialValues({});
        final store = _FailingCacheStore();
        SharedPreferencesStorePlatform.instance = store;
        final prefs = await SharedPreferences.getInstance();
        final relay = FakeRelay();
        final config = RelayConfig(baseUrl: origin, nsec: nsec);
        final service = MeSync(relay, config, prefs: prefs);
        final old = signed(timestamp: 1);
        await service.cache(old);
        final fresh = signed(
          timestamp: 2,
          groups: const MeGroups(
            channels: ['11111111-1111-4111-8111-111111111111'],
          ),
        );
        relay.events = [fresh];
        store.failure = failure;
        final container = ProviderContainer(
          overrides: [meSyncProvider.overrideWithValue(service)],
        );
        addTearDown(container.dispose);
        final failed = Completer<void>();
        container.listen(meGroupsProvider, (_, _) {});
        container.listen(meOfflineSaveWarningProvider, (_, next) {
          if (next.value == true && !failed.isCompleted) failed.complete();
        });
        await failed.future;
        expect(container.read(meGroupsProvider).hasError, isFalse);
        expect(container.read(meOfflineSaveWarningProvider).value, isTrue);
        expect(service.cachedSnapshot()?.event?.id, old.id);
        store.failure = null;
        container.invalidate(meGroupsProvider);
        final restored = Completer<void>();
        final subscription = container.listen(meOfflineSaveWarningProvider, (
          _,
          next,
        ) {
          if (next.value == false && !restored.isCompleted) restored.complete();
        });
        await restored.future;
        subscription.close();
        await prefs.reload();
        final restarted = MeSync(
          FakeRelay(),
          config,
          prefs: prefs,
          connected: false,
        );
        expect(restarted.cachedSnapshot()?.event?.id, fresh.id);
      },
    );
  }
  test(
    'offline relaunch emits encrypted cached organization and isolates accounts',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = RelayConfig(baseUrl: origin, nsec: nsec);
      final online = MeSync(FakeRelay(), config, prefs: prefs);
      const groups = MeGroups(
        groups: [
          {'id': 'project', 'name': 'Private project', 'defaultTemplateId': ''},
        ],
      );
      await online.cache(signed(groups: groups, timestamp: 3));
      await online.cache(signed(timestamp: 2));
      await online.cache(null);
      expect(
        prefs.getString(online.cacheKey),
        isNot(contains('Private project')),
      );
      final offline = MeSync(
        FakeRelay(),
        config,
        prefs: prefs,
        connected: false,
      );
      final container = ProviderContainer(
        overrides: [meSyncProvider.overrideWithValue(offline)],
        retry: (_, _) => null,
      );
      addTearDown(container.dispose);
      container.listen(meGroupsProvider, (_, _) {});
      expect(
        (await container.read(meGroupsProvider.future)).groups.single['name'],
        'Private project',
      );
      await Future<void>.delayed(Duration.zero);
      expect(container.read(meGroupsProvider).hasError, isFalse);
      for (final other in [
        RelayConfig(baseUrl: 'https://other.test', nsec: nsec),
        RelayConfig(
          baseUrl: origin,
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '0' * 63 + '2',
          ),
        ),
      ]) {
        expect(
          MeSync(
            FakeRelay(),
            other,
            prefs: prefs,
            connected: false,
          ).cachedSnapshot(),
          isNull,
        );
      }
    },
  );
  test(
    'corrupt cache refreshes online and persists the verified replacement',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay()..events = [signed(timestamp: 5)];
      final service = MeSync(
        relay,
        RelayConfig(baseUrl: origin, nsec: nsec),
        prefs: prefs,
      );
      await prefs.setString(service.cacheKey, 'corrupt');
      final container = ProviderContainer(
        overrides: [meSyncProvider.overrideWithValue(service)],
        retry: (_, _) => null,
      );
      addTearDown(container.dispose);
      container.listen(meGroupsProvider, (_, _) {});
      expect((await container.read(meGroupsProvider.future)).channels, isEmpty);
      await Future<void>.delayed(Duration.zero);
      expect(service.cachedSnapshot()?.event?.id, relay.events.single.id);
    },
  );
  test(
    'empty organization survives relaunch without relay connectivity',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = RelayConfig(baseUrl: origin, nsec: nsec);
      await MeSync(FakeRelay(), config, prefs: prefs).cache(null);
      final offline = MeSync(
        FakeRelay(),
        config,
        prefs: prefs,
        connected: false,
      );
      expect(offline.cachedSnapshot()?.groups.channels, isEmpty);
    },
  );
  test(
    'cached organization emits before refresh and survives a closed subscription',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final response = Completer<List<NostrEvent>>();
      final relay = FakeRelay()..pendingRead = response.future;
      final service = MeSync(
        relay,
        RelayConfig(baseUrl: origin, nsec: nsec),
        prefs: prefs,
      );
      final old = signed(timestamp: 1);
      await service.cache(old);
      final container = ProviderContainer(
        overrides: [meSyncProvider.overrideWithValue(service)],
        retry: (_, _) => null,
      );
      addTearDown(container.dispose);
      container.listen(meGroupsProvider, (_, _) {});
      expect((await container.read(meGroupsProvider.future)).groups, isEmpty);
      final fresh = signed(
        timestamp: 2,
        groups: const MeGroups(
          groups: [
            {'id': 'new', 'name': 'From desktop', 'defaultTemplateId': ''},
          ],
        ),
      );
      response.complete([fresh]);
      await Future<void>.delayed(Duration.zero);
      expect(
        container.read(meGroupsProvider).value?.groups.single['name'],
        'From desktop',
      );
      expect(service.cachedSnapshot()?.event?.id, fresh.id);
      relay.closeSubscription!('offline');
      await Future<void>.delayed(Duration.zero);
      expect(service.syncClosed.value, isTrue);
      expect(container.read(meGroupsProvider).hasError, isFalse);
      expect(
        container.read(meGroupsProvider).value?.groups.single['name'],
        'From desktop',
      );
    },
  );
  test('verifies signature, account and encrypted community envelope', () {
    final sync = MeSync(FakeRelay(), RelayConfig(baseUrl: origin, nsec: nsec));
    expect(sync.decode(signed()).channels, isEmpty);
    expect(
      () => sync.decode(signed(community: 'https://other.test')),
      throwsFormatException,
    );
    expect(
      () => sync.decode(
        signed(
          signingKey:
              '0000000000000000000000000000000000000000000000000000000000000002',
        ),
      ),
      throwsFormatException,
    );
    final tampered = signed().toJson()..['sig'] = '0' * 128;
    expect(() => sync.decode(NostrEvent.fromJson(tampered)), throwsA(anything));
  });
  test('equal timestamp selects the lowest event ID', () {
    final a = signed(timestamp: 2);
    final b = signed(timestamp: 2);
    expect(meEventIsNewer(a, b), a.id.compareTo(b.id) < 0);
    expect(meEventIsNewer(signed(timestamp: 3), a), isTrue);
  });
  test('scope disposal during preparation prevents publication', () async {
    final relay = FakeRelay();
    final sync = MeSync(relay, RelayConfig(baseUrl: origin, nsec: nsec));
    relay.beforeRead = () => sync.disposed = true;
    await expectLater(sync.update((v) => v), throwsStateError);
    expect(relay.writes, 0);
  });
  test('concurrent desktop change blocks stale replacement', () async {
    final relay = FakeRelay()..events = [signed()];
    final sync = MeSync(relay, RelayConfig(baseUrl: origin, nsec: nsec));
    var reads = 0;
    relay.beforeRead = () {
      if (++reads == 2) relay.events = [signed(timestamp: 2)];
    };
    await expectLater(sync.update((v) => v), throwsStateError);
    expect(relay.writes, 0);
  });
  test(
    'unconfirmed save survives service restart and never silently replaces it',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay()..loseAcknowledgment = true;
      final config = RelayConfig(baseUrl: origin, nsec: nsec);
      final first = MeSync(relay, config, prefs: prefs);
      await expectLater(
        first.update((v) => v),
        throwsA(isA<TimeoutException>()),
      );
      final pending = prefs.getString(first.pendingKey);
      expect(pending, isNotNull);
      expect(
        jsonDecode(pending!)['event']['content'],
        isNot(contains('groups')),
      );
      final second = MeSync(relay, config, prefs: prefs);
      await expectLater(second.update((v) => v), throwsStateError);
      expect(relay.writes, 1);
      expect(prefs.getString(first.pendingKey), isNull);
    },
  );
  test(
    'superseded pending save rebases local additions after restart',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      final config = RelayConfig(baseUrl: origin, nsec: nsec);
      final sync = MeSync(relay, config, prefs: prefs);
      const base = MeGroups(
        groups: [
          {'id': 'old', 'name': 'Original', 'defaultTemplateId': ''},
        ],
      );
      const local = MeGroups(
        groups: [
          {'id': 'old', 'name': 'Original', 'defaultTemplateId': ''},
          {'id': 'local', 'name': 'Phone project', 'defaultTemplateId': ''},
        ],
      );
      const remote = MeGroups(
        groups: [
          {'id': 'old', 'name': 'Desktop rename', 'defaultTemplateId': ''},
          {'id': 'remote', 'name': 'Desktop project', 'defaultTemplateId': ''},
        ],
      );
      await prefs.setString(
        sync.pendingKey,
        jsonEncode({
          'event': signed(groups: local, timestamp: 2).toJson(),
          'base': signed(groups: base, timestamp: 1).toJson(),
        }),
      );
      relay.events = [signed(groups: remote, timestamp: 3)];
      final restarted = MeSync(relay, config, prefs: prefs);
      var newIntentRan = false;
      await expectLater(
        restarted.update((v) {
          newIntentRan = true;
          return v;
        }),
        throwsStateError,
      );
      expect(newIntentRan, isFalse);
      expect(
        restarted.decode(relay.events.single).groups.map((g) => g['name']),
        ['Desktop rename', 'Desktop project', 'Phone project'],
      );
      expect(prefs.getString(sync.pendingKey), isNull);
      expect(relay.writes, 1);
    },
  );
  test('conflicting edits remain durable until explicit discard', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final relay = FakeRelay();
    final sync = MeSync(
      relay,
      RelayConfig(baseUrl: origin, nsec: nsec),
      prefs: prefs,
    );
    MeGroups group(String name) => MeGroups(
      groups: [
        {'id': 'project', 'name': name, 'defaultTemplateId': ''},
      ],
    );
    final stored = jsonEncode({
      'event': signed(groups: group('Phone'), timestamp: 2).toJson(),
      'base': signed(groups: group('Original')).toJson(),
    });
    await prefs.setString(sync.pendingKey, stored);
    final remote = signed(groups: group('Desktop'), timestamp: 3);
    relay.events = [remote];
    await expectLater(
      sync.update((v) => v),
      throwsA(isA<MeRecoveryConflict>()),
    );
    expect(prefs.getString(sync.pendingKey), stored);
    expect(relay.writes, 0);
    await expectLater(sync.discardPending('stale snapshot'), throwsStateError);
    expect(prefs.getString(sync.pendingKey), stored);
    await sync.discardPending(stored);
    await sync.update(
      (v) => MeGroups(
        groups: [
          ...v.groups,
          {'id': 'new', 'name': 'New', 'defaultTemplateId': ''},
        ],
      ),
    );
    expect(sync.decode(relay.events.single).groups.map((g) => g['name']), [
      'Desktop',
      'New',
    ]);
    expect(prefs.getString(sync.pendingKey), isNull);
  });
}
