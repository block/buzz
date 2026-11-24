import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/profile/profile_event_parser.dart';
import 'package:buzz/shared/profile/profile_snapshot.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/profile/verified_profile_owner.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:crypto/crypto.dart' as crypto;
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
const _agentSecret =
    '0000000000000000000000000000000000000000000000000000000000000003';
const _ownerSecret =
    '0000000000000000000000000000000000000000000000000000000000000004';

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

  test('signed owner evidence persists, backfills and is replaced', () async {
    final agent = derivePublicKeyHex(_agentSecret);
    final owner = derivePublicKeyHex(_ownerSecret);
    final attested = _signedProfile(name: 'Helper', createdAt: 10);
    final storage = ProfileSnapshotStorage(prefs);
    final me = pubkeyFromNsec(_nsec)!;
    final key = ProfileSnapshotStorage.key(me, _relay);
    String? verifiedOwner(ProviderContainer scope) =>
        scope.read(verifiedProfileOwnerProvider(agent));

    // A snapshot from before evidence was kept: the name and the agent badge
    // still render, but the owner is unverified, so it grants no permission.
    await prefs.setString(
      key,
      jsonEncode({
        'v': 1,
        'profiles': [
          {
            'pubkey': agent,
            'event_id': attested.id,
            'created_at': attested.createdAt,
            'display_name': 'Helper',
            'owner': owner,
          },
        ],
      }),
    );
    final upgrade = _ProfileSession([
      Future.value([attested]),
    ]);
    final launch1 = cacheScope(upgrade);
    final cache1 = launch1.read(userCacheProvider.notifier);
    expect(cache1.get(agent)?.displayName, 'Helper');
    expect(cache1.state[agent]?.ownerPubkey, owner);
    expect(verifiedOwner(launch1), isNull);
    // Revalidation returns the same event, which fills in its evidence.
    // Wait for that rather than for a time: parsing runs on an isolate.
    final backfilled = Completer<void>();
    launch1.listen(verifiedProfileOwnerProvider(agent), (_, next) {
      if (next == owner && !backfilled.isCompleted) backfilled.complete();
    });
    await backfilled.future.timeout(const Duration(seconds: 10));
    expect(upgrade.requests, 1);
    expect(verifiedOwner(launch1), owner);
    launch1.dispose();
    await pumpEventQueue();

    // The evidence is saved with the profile, so an offline launch verifies
    // the owner before the relay answers.
    final stored = jsonDecode(prefs.getString(key)!) as Map<String, dynamic>;
    final entry = (stored['profiles'] as List).single as Map<String, dynamic>;
    expect((entry['signed'] as Map)['id'], attested.id);
    final offline = _ProfileSession([], fail: true);
    final launch2 = cacheScope(offline);
    expect(verifiedOwner(launch2), owner);
    expect(offline.requests, 0);

    // A newer profile without the attestation clears the owner at once.
    final cache2 = launch2.read(userCacheProvider.notifier);
    final removed = NostrEvent.fromJson(
      signNostrEvent(
        kind: 0,
        content: jsonEncode({'name': 'Helper'}),
        secretKey: _agentSecret,
        createdAt: 20,
      ).toMap(),
    );
    cache2.cacheProfileEvent(removed);
    expect(verifiedOwner(launch2), isNull);
    expect(cache2.state[agent]?.ownerEvidence, isNull);
    launch2.dispose();
    await pumpEventQueue();
    expect(
      ((jsonDecode(prefs.getString(key)!) as Map)['profiles'] as List).single,
      isNot(contains('signed')),
    );

    // Stored evidence is checked, not trusted: a tampered copy keeps the
    // name but verifies no owner, and another event's copy is dropped.
    Map<String, Object?> withEvidence(NostrEvent signed) => {
      'pubkey': agent,
      'event_id': attested.id,
      'created_at': attested.createdAt,
      'display_name': 'Helper',
      'owner': owner,
      'signed': signed.toJson(),
    };
    final tampered = NostrEvent.fromJson({
      ...attested.toJson(),
      'content': jsonEncode({'name': 'Impostor'}),
    });
    final other = _signedProfile(name: 'Helper', createdAt: 11);
    for (final signed in [tampered, other]) {
      await prefs.setString(
        key,
        jsonEncode({
          'v': 1,
          'profiles': [withEvidence(signed)],
        }),
      );
      final restored = storage.read(me, _relay).single.profile;
      expect(restored.displayName, 'Helper');
      expect(
        restored.ownerEvidence == null
            ? null
            : verifiedEvidenceOwner(restored.ownerEvidence!, agent),
        isNull,
      );
    }
    expect(storage.read(me, _relay).single.profile.ownerEvidence, isNull);

    // So is an entry stored as its signed event alone: a copy with altered
    // content or a bad signature restores no owner.
    final badSignature = NostrEvent.fromJson({
      ...attested.toJson(),
      'sig': '0' * 128,
    });
    for (final signed in [tampered, badSignature]) {
      await prefs.setString(
        key,
        jsonEncode({
          'v': 1,
          'profiles': [
            {'owner': owner, 'signed': signed.toJson()},
          ],
        }),
      );
      final restore = cacheScope(_ProfileSession([], fail: true));
      expect(restore.read(userCacheProvider)[agent]?.ownerPubkey, owner);
      expect(verifiedOwner(restore), isNull);
      restore.dispose();
      await pumpEventQueue();
    }

    // An attested profile is stored once, as its signed event, so one
    // whose fields and event together would pass the bound keeps both.
    final large = _signedProfile(name: 'Helper', createdAt: 12, pad: 9000);
    final parsed = parseProfileEvent(large);
    expect(parsed.profile.ownerEvidence, isNotNull);
    await storage.write(storage.capture(me, _relay), [parsed]);
    final kept = storage.read(me, _relay).single;
    expect(kept.eventId, large.id);
    expect(kept.createdAt, large.createdAt);
    expect(kept.profile.displayName, 'Helper');
    expect(kept.profile.about, hasLength(9000));
    expect(kept.profile.ownerPubkey, owner);
    expect(verifiedEvidenceOwner(kept.profile.ownerEvidence!, agent), owner);

    // A signed event past the bound is left out; the name is kept and the
    // owner is verified again after the next fetch.
    final oversized = _signedProfile(
      name: 'Helper',
      createdAt: 13,
      banner: maxProfileSnapshotEntryBytes,
    );
    await storage.write(storage.capture(me, _relay), [
      parseProfileEvent(oversized),
    ]);
    final named = storage.read(me, _relay).single;
    expect(named.eventId, oversized.id);
    expect(named.profile.displayName, 'Helper');
    expect(named.profile.ownerPubkey, owner);
    expect(named.profile.ownerEvidence, isNull);
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

/// The agent's kind:0, signed, with a valid owner attestation.
NostrEvent _signedProfile({
  required String name,
  required int createdAt,
  int pad = 0,
  int banner = 0,
}) {
  final agent = derivePublicKeyHex(_agentSecret);
  final digest = crypto.sha256.convert(utf8.encode('nostr:agent-auth:$agent:'));
  return NostrEvent.fromJson(
    signNostrEvent(
      kind: 0,
      content: jsonEncode({
        'name': name,
        if (pad > 0) 'about': 'x' * pad,
        if (banner > 0) 'banner': 'x' * banner,
      }),
      secretKey: _agentSecret,
      createdAt: createdAt,
      tags: [
        [
          'auth',
          derivePublicKeyHex(_ownerSecret),
          '',
          signSchnorrHex(secretKey: _ownerSecret, messageHex: '$digest'),
        ],
      ],
    ).toMap(),
  );
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
