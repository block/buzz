import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('delayed absence seeds the intervening local edit', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final history = Completer<List<NostrEvent>>();
    final session = _ThemeRelaySession(
      keys.nsec,
      keys.public,
      historyFuture: history.future,
    );
    final storage = CommunityThemeStorage(prefs);
    final container = ProviderContainer(
      overrides: [
        communityThemeStorageProvider.overrideWithValue(storage),
        relayConfigProvider.overrideWith(() => _RelayConfig(keys.nsec)),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    container.listen(communityThemeProvider, (_, _) {}, fireImmediately: true);

    container.read(communityThemeProvider.notifier).setTheme('dracula');
    history.complete([]);
    await _waitUntil(() => session.published != null);

    expect(container.read(communityThemeProvider).theme, 'dracula');
    expect(
      storage.read(keys.public, 'https://relay.example')?.theme,
      'dracula',
    );
  });

  test('edit during delayed absence seed wins durable state', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final history = Completer<List<NostrEvent>>();
    final session = _ThemeRelaySession(
      keys.nsec,
      keys.public,
      historyFuture: history.future,
    );
    final storage = _DelayedThemeStorage(prefs);
    final container = ProviderContainer(
      overrides: [
        communityThemeStorageProvider.overrideWithValue(storage),
        relayConfigProvider.overrideWith(() => _RelayConfig(keys.nsec)),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    container.listen(communityThemeProvider, (_, _) {}, fireImmediately: true);

    history.complete([]);
    await storage.cacheWriteStarted.future;
    container.read(communityThemeProvider.notifier).setTheme('dracula');
    storage.allowCacheWrite.complete();
    storage.allowOutboxWrite.complete();
    await _waitUntil(() => session.published != null);

    expect(container.read(communityThemeProvider).theme, 'dracula');
    expect(
      storage.read(keys.public, 'https://relay.example')?.theme,
      'dracula',
    );
    final privateHex = nostr.Nip19.decode(payload: keys.nsec).data;
    final key = getConversationKey(privateHex, keys.public);
    expect(
      jsonDecode(nip44Decrypt(key, session.published!.content))['theme'],
      'dracula',
    );
  });

  test(
    'local edit stays authoritative before persistence through exact ack',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      final session = _ThemeRelaySession(keys.nsec, keys.public);
      final storage = _DelayedThemeStorage(prefs);
      final container = ProviderContainer(
        overrides: [
          communityThemeStorageProvider.overrideWithValue(storage),
          relayConfigProvider.overrideWith(() => _RelayConfig(keys.nsec)),
          relaySessionProvider.overrideWith(() => session),
        ],
      );
      addTearDown(container.dispose);

      final subscription = container.listen(
        communityThemeProvider,
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(subscription.close);
      await session.subscribed.future;

      final notifier = container.read(communityThemeProvider.notifier);
      notifier.setTheme('dracula');
      const local = CommunityThemePreference(
        theme: 'dracula',
        accent: '#3b82f6',
        followSystem: true,
      );
      expect(container.read(communityThemeProvider), local);

      session.emit(session.remoteEvent(theme: 'houston', id: 'remote-z'));
      expect(container.read(communityThemeProvider), local);

      storage.allowCacheWrite.complete();
      await storage.outboxWriteStarted.future;
      session.emit(session.remoteEvent(theme: 'solarized', id: 'remote-a'));
      expect(container.read(communityThemeProvider), local);

      storage.allowOutboxWrite.complete();
      await _waitUntil(() => session.published != null);
      expect(container.read(communityThemeProvider), local);

      session.emit(session.published!);
      await _pumpEventQueue();
      expect(container.read(communityThemeProvider), local);
      expect(storage.readOutbox(keys.public, 'https://relay.example'), isNull);
    },
  );

  test(
    'provider rebuild preserves delayed local edit and publishes on replacement manager',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      final session = _ThemeRelaySession(keys.nsec, keys.public);
      final storage = _DelayedThemeStorage(prefs);
      final container = ProviderContainer(
        overrides: [
          communityThemeStorageProvider.overrideWithValue(storage),
          relayConfigProvider.overrideWith(() => _RelayConfig(keys.nsec)),
          relaySessionProvider.overrideWith(() => session),
        ],
      );
      addTearDown(container.dispose);
      final subscription = container.listen(
        communityThemeProvider,
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(subscription.close);
      await session.subscribed.future;

      container.read(communityThemeProvider.notifier).setTheme('dracula');
      expect(container.read(communityThemeProvider).theme, 'dracula');
      await storage.cacheWriteStarted.future;

      session.setStatus(SessionStatus.reconnecting);
      await _pumpEventQueue();
      expect(container.read(communityThemeProvider).theme, 'dracula');
      session.setStatus(SessionStatus.connected);
      await _waitUntil(() => session.subscribeCalls == 2);
      expect(container.read(communityThemeProvider).theme, 'dracula');

      storage.allowCacheWrite.complete();
      storage.allowOutboxWrite.complete();
      await _waitUntil(() => session.published != null);

      final privateHex = nostr.Nip19.decode(payload: keys.nsec).data;
      final key = getConversationKey(privateHex, keys.public);
      expect(
        jsonDecode(nip44Decrypt(key, session.published!.content))['theme'],
        'dracula',
      );
      expect(container.read(communityThemeProvider).theme, 'dracula');
    },
  );

  test('complete preference writes one cache and outbox snapshot', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final session = _ThemeRelaySession(keys.nsec, keys.public);
    final storage = _RecordingThemeStorage(prefs);
    final container = ProviderContainer(
      overrides: [
        communityThemeStorageProvider.overrideWithValue(storage),
        relayConfigProvider.overrideWith(() => _RelayConfig(keys.nsec)),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    final subscription = container.listen(
      communityThemeProvider,
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(subscription.close);

    await _waitUntil(() => storage.cacheWrites.isNotEmpty);
    storage.cacheWrites.clear();
    storage.outboxWrites.clear();

    const selection = CommunityThemePreference(
      theme: 'dracula',
      accent: '#22c55e',
      followSystem: false,
    );
    container.read(communityThemeProvider.notifier).setPreference(selection);
    await _waitUntil(() => storage.outboxWrites.isNotEmpty);

    expect(storage.cacheWrites, [selection]);
    expect(storage.outboxWrites, [selection]);
    expect(container.read(communityThemeProvider), selection);
  });
}

class _RecordingThemeStorage extends CommunityThemeStorage {
  _RecordingThemeStorage(super.prefs);

  final cacheWrites = <CommunityThemePreference>[];
  final outboxWrites = <CommunityThemePreference>[];

  @override
  Future<bool> write(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) async {
    cacheWrites.add(preference);
    return super.write(pubkey, relayUrl, preference);
  }

  @override
  Future<bool> writeOutbox(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) async {
    outboxWrites.add(preference);
    return super.writeOutbox(pubkey, relayUrl, preference);
  }
}

class _DelayedThemeStorage extends CommunityThemeStorage {
  _DelayedThemeStorage(super.prefs);

  final allowCacheWrite = Completer<void>();
  final allowOutboxWrite = Completer<void>();
  final cacheWriteStarted = Completer<void>();
  final outboxWriteStarted = Completer<void>();

  @override
  Future<bool> write(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) async {
    if (!cacheWriteStarted.isCompleted) cacheWriteStarted.complete();
    await allowCacheWrite.future;
    return super.write(pubkey, relayUrl, preference);
  }

  @override
  Future<bool> writeOutbox(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) async {
    if (!outboxWriteStarted.isCompleted) outboxWriteStarted.complete();
    await allowOutboxWrite.future;
    return super.writeOutbox(pubkey, relayUrl, preference);
  }
}

class _RelayConfig extends RelayConfigNotifier {
  _RelayConfig(this.nsec);

  final String nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _ThemeRelaySession extends RelaySessionNotifier {
  _ThemeRelaySession(this.nsec, this.pubkey, {this.historyFuture});

  final String nsec;
  final String pubkey;
  final Future<List<NostrEvent>>? historyFuture;
  final subscribed = Completer<void>();
  int subscribeCalls = 0;
  void Function(NostrEvent)? _listener;
  NostrEvent? published;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => historyFuture ?? [remoteEvent(theme: 'buzz', id: 'initial')];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    subscribeCalls++;
    _listener = onEvent;
    if (!subscribed.isCompleted) subscribed.complete();
    return () => _listener = null;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published = event;
    return event;
  }

  void emit(NostrEvent event) => _listener?.call(event);

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  NostrEvent remoteEvent({required String theme, required String id}) {
    final privateHex = nostr.Nip19.decode(payload: nsec).data;
    final key = getConversationKey(privateHex, pubkey);
    final preference = CommunityThemePreference(
      theme: theme,
      accent: '#3b82f6',
      followSystem: true,
    );
    return NostrEvent(
      id: id,
      pubkey: pubkey,
      createdAt: 1,
      kind: 30078,
      tags: const [
        ['d', communityThemeDTag],
        ['t', communityThemeDTag],
      ],
      content: nip44Encrypt(key, jsonEncode(preference.toJson())),
      sig: 'sig',
    );
  }
}

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

Future<void> _waitUntil(bool Function() condition) async {
  final deadline = DateTime.now().add(const Duration(seconds: 3));
  while (!condition()) {
    if (DateTime.now().isAfter(deadline)) fail('condition not met');
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
}
