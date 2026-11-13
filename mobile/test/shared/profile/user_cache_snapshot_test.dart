import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/profile/profile_snapshot.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

const _relay = 'https://relay.test';
final _nsec = nostr.Nip19.encode(
  prefix: nostr.Nip19Prefix.nsec,
  data: '1' * 64,
);
const _alice = 'a11ce';

void main() {
  late SharedPreferences prefs;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
  });

  ProviderContainer cacheScope(_ProfileSession session) {
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(_Config.new),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('profiles persist across launches and revalidate once', () async {
    // First launch: nothing on disk, so the name loads from the relay.
    final first = _ProfileSession([
      Future.value([_profile('v1', 1, 'Alice')]),
    ]);
    final launch1 = cacheScope(first);
    final cache1 = launch1.read(userCacheProvider.notifier);
    expect(cache1.get(_alice), isNull);
    await cache1.preload(const [_alice]);
    expect(cache1.state[_alice]?.displayName, 'Alice');
    // Leaving the scope flushes the pending snapshot write.
    launch1.dispose();
    await pumpEventQueue();

    // Second launch, relay unreachable: the stored name renders immediately
    // and the failed revalidation leaves it in place for another attempt.
    final offline = _ProfileSession([], fail: true);
    final launch2 = cacheScope(offline);
    final cache2 = launch2.read(userCacheProvider.notifier);
    expect(cache2.get(_alice)?.displayName, 'Alice');
    expect(await cache2.preload(const [_alice]), isTrue);
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(offline.requests, 1);
    expect(cache2.state[_alice]?.displayName, 'Alice');
    cache2.get(_alice);
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(offline.requests, 2, reason: 'a failed revalidation retries');
    launch2.dispose();

    // Third launch, relay reachable: one revalidation picks up the rename,
    // and later lookups are served from memory.
    final held = Completer<List<NostrEvent>>();
    final online = _ProfileSession([
      held.future,
      Future.value([_profile('v2', 2, 'Alice Renamed')]),
    ]);
    final launch3 = cacheScope(online);
    final cache3 = launch3.read(userCacheProvider.notifier);
    expect(cache3.get(_alice)?.displayName, 'Alice');
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(online.requests, 1);
    // A gate resolving while that revalidation is on the wire must not trust
    // the restored profile; it confirms with the relay itself.
    final resolved = await cache3.resolve(const [_alice]);
    expect(resolved[_alice]?.displayName, 'Alice Renamed');
    expect(online.requests, 2);
    held.complete(const []);
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(cache3.state[_alice]?.displayName, 'Alice Renamed');
    cache3.get(_alice);
    await cache3.preload(const [_alice]);
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(online.requests, 2);
    launch3.dispose();
    await pumpEventQueue();

    // Signing out of the community deletes the snapshot.
    final storage = ProfileSnapshotStorage(prefs);
    final pubkey = pubkeyFromNsec(_nsec)!;
    expect(
      storage.read(pubkey, _relay).single.profile.displayName,
      'Alice Renamed',
    );
    await storage.remove(pubkey, _relay);
    final launch4 = cacheScope(_ProfileSession([]));
    expect(launch4.read(userCacheProvider), isEmpty);
  });

  test('restored profiles revalidate with since their last check', () async {
    const bob = 'b0b';
    const carol = 'ca401';
    // First launch: both load in full and record when they were checked.
    final before = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final first = _ProfileSession([
      Future.value([_profile('v1', 1, 'Alice')]),
    ]);
    final launch1 = cacheScope(first);
    await launch1.read(userCacheProvider.notifier).preload(const [_alice]);
    final after = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    expect(first.filters.single.since, isNull);
    launch1.dispose();
    await pumpEventQueue();
    // A profile stored before check times existed has no `checked_at`.
    final storage = ProfileSnapshotStorage(prefs);
    final pubkey = pubkeyFromNsec(_nsec)!;
    await storage.write(
      storage.capture(pubkey, _relay),
      [
        ...storage
            .read(pubkey, _relay)
            .map(
              (p) => (
                profile: p.profile,
                createdAt: p.createdAt,
                eventId: p.eventId,
              ),
            ),
        _parsed(bob, 1, 'Bob'),
      ],
      checkedAt: {_alice: storage.read(pubkey, _relay).single.checkedAt},
    );

    // Next launch: the checked profile asks only for changes since its check
    // (less the skew margin); the unchecked one fetches in full. An empty
    // answer confirms the stored profile.
    final second = _ProfileSession([]);
    final launch2 = cacheScope(second);
    final cache2 = launch2.read(userCacheProvider.notifier);
    expect(await cache2.preload(const [_alice, bob]), isTrue);
    await _until(() => second.requests == 2);
    final full = second.filters.singleWhere((f) => f.since == null);
    final changed = second.filters.singleWhere((f) => f.since != null);
    expect(full.authors, [bob]);
    expect(changed.authors, [_alice]);
    expect(changed.since, inInclusiveRange(before - 3600, after - 3600));
    expect(cache2.state[_alice]?.displayName, 'Alice');
    // A later lookup doesn't re-ask: awaiting an uncached profile drains the
    // batch Alice would have joined.
    cache2.get(_alice);
    expect(await cache2.preload(const [carol]), isTrue);
    expect(second.requests, 3);
    expect(second.filters.last.authors, [carol]);
    launch2.dispose();
    await pumpEventQueue();
    // The profile that had no check time gets one stored.
    expect(
      storage.read(pubkey, _relay).map((p) => p.checkedAt > 0),
      everyElement(isTrue),
    );

    // A launch that only confirms recently checked profiles leaves the
    // snapshot untouched rather than rewriting it per lookup.
    final hourAgo = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 3600;
    final restored = storage.read(pubkey, _relay);
    await storage.write(
      storage.capture(pubkey, _relay),
      [
        for (final p in restored)
          (profile: p.profile, createdAt: p.createdAt, eventId: p.eventId),
      ],
      checkedAt: {for (final p in restored) p.profile.pubkey: hourAgo},
    );
    final key = ProfileSnapshotStorage.key(pubkey, _relay);
    final stored = prefs.getString(key);
    final third = _ProfileSession([]);
    final launch3 = cacheScope(third);
    final cache3 = launch3.read(userCacheProvider.notifier);
    expect(await cache3.preload(const [_alice, bob]), isTrue);
    // Awaiting an uncached profile drains the revalidation before it.
    expect(await cache3.preload(const [carol]), isTrue);
    expect(third.filters.where((f) => f.since != null), hasLength(1));
    launch3.dispose();
    await pumpEventQueue();
    expect(prefs.getString(key), stored);
  });

  test('snapshot scopes are per identity and relay, and bounded', () async {
    final storage = ProfileSnapshotStorage(prefs);
    final huge = _parsed('huge', 3, 'x' * maxProfileSnapshotEntryBytes);
    final token = storage.capture('ME', 'wss://relay.test');
    expect(
      await storage.write(token, [_parsed(_alice, 1, 'Alice'), huge]),
      isTrue,
    );
    // The wss:// and https:// onboarding paths share a scope.
    expect(storage.read('me', _relay).map((p) => p.profile.pubkey), [_alice]);
    expect(storage.read('someone-else', _relay), isEmpty);

    // A removal retires writers captured before it.
    await storage.remove('me', _relay);
    expect(await storage.write(token, [_parsed(_alice, 1, 'Alice')]), isFalse);
    expect(storage.read('me', _relay), isEmpty);

    // Corrupt data is discarded.
    await prefs.setString(ProfileSnapshotStorage.key('me', _relay), '{');
    expect(storage.read('me', _relay), isEmpty);
    expect(prefs.getString(ProfileSnapshotStorage.key('me', _relay)), isNull);
  });
}

/// Waits on state rather than a fixed sleep.
Future<void> _until(bool Function() done) async {
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('condition never held');
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
  await pumpEventQueue();
}

NostrEvent _profile(String id, int createdAt, String name) => NostrEvent(
  id: id,
  pubkey: _alice,
  createdAt: createdAt,
  kind: 0,
  tags: const [],
  content: jsonEncode({'name': name}),
  sig: 'sig',
);

({UserProfile profile, int createdAt, String eventId}) _parsed(
  String pubkey,
  int createdAt,
  String name,
) => (
  profile: UserProfile(pubkey: pubkey, displayName: name),
  createdAt: createdAt,
  eventId: 'e-$pubkey',
);

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() => RelayConfig(baseUrl: _relay, nsec: _nsec);
}

class _ProfileSession extends RelaySessionNotifier {
  _ProfileSession(this._results, {this.fail = false});

  final List<Future<List<NostrEvent>>> _results;
  final bool fail;
  int requests = 0;
  final filters = <NostrFilter>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    requests++;
    filters.add(filter);
    if (fail) throw StateError('offline');
    return _results.isEmpty ? const [] : await _results.removeAt(0);
  }
}
