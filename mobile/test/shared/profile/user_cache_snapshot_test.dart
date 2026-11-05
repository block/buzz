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

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    requests++;
    if (fail) throw StateError('offline');
    return _results.isEmpty ? const [] : await _results.removeAt(0);
  }
}
