import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'dart:convert';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const relay = 'wss://community.example';
  // Public test vectors, never real user signing material.
  final key = nostr.Keys('1'.padLeft(64, '0'));
  final other = nostr.Keys('2'.padLeft(64, '0'));
  late _Storage storage;
  late List<Community> probes;
  late SelfHostedEntry entry;
  var current = true;
  var refuseProbe = false;

  SelfHostedEntry makeEntry() => SelfHostedEntry(
    storage: storage,
    identities: DeviceIdentitySource(storage: storage),
    accountStore: AccountIdentityStore(storage: storage),
    probe: (community) async {
      // The durable key must already exist before any network operation.
      expect(storage.data.values, contains(community.nsec));
      probes.add(community);
      if (refuseProbe) throw StateError('Connection refused');
    },
  );

  setUp(() {
    storage = _Storage();
    probes = [];
    current = true;
    refuseProbe = false;
    entry = makeEntry();
  });

  Future<Community> prepare({
    String address = relay,
    String? nsec,
    String? savedPubkey,
    bool create = false,
  }) => entry.prepare(
    relay: address,
    name: 'Community',
    nsec: nsec,
    savedPubkey: savedPubkey,
    create: create,
    isCurrent: () => current,
  );

  void expectUntouched() {
    expect(storage.writes, 0);
    expect(probes, isEmpty);
  }

  test(
    'reserved hosted domains refuse imported and generated identities',
    () async {
      for (final address in [
        'wss://team.communities.buzz.xyz',
        'wss://team.communities.public.builderlabstaging.xyz',
        'wss://BUILDERLAB.XYZ.',
      ]) {
        if (!HostedEnvironment.requiresAuth(Uri.parse(address).host)) continue;
        await expectLater(
          prepare(address: address, nsec: key.nsec),
          throwsStateError,
        );
        await expectLater(
          prepare(address: address, create: true),
          throwsStateError,
        );
      }
      expectUntouched();
      expect(storage.data, isEmpty);
    },
  );

  test('invalid addresses and invalid nsec never persist or probe', () async {
    for (final address in [
      'ws://community.example',
      'wss://user@community.example',
      'wss://community.example/path',
      'wss://community.example?token=value',
      'wss://community.example#fragment',
    ]) {
      await expectLater(
        prepare(address: address, nsec: key.nsec),
        throwsFormatException,
      );
    }
    for (final nsec in ['invalid', key.public, key.npub]) {
      await expectLater(
        prepare(nsec: nsec),
        throwsA(AccountIdentityFailure.invalidKey),
      );
    }
    expectUntouched();
    expect(storage.data, isEmpty);
  });

  test(
    'failed probe retains generated key across a new service and retry',
    () async {
      refuseProbe = true;
      await expectLater(prepare(create: true), throwsStateError);
      expect(storage.writes, 1);
      final persisted = Map<String, String>.of(storage.data);
      final first = probes.single;
      expect(first.pubkey, AccountIdentityStore.publicKeyForNsec(first.nsec!));
      refuseProbe = false;
      entry = makeEntry();
      final retried = await prepare(create: true);
      expect(retried.pubkey, first.pubkey);
      expect(retried.nsec, first.nsec);
      expect(storage.data, persisted);
      expect(storage.writes, 1);
      expect(probes, hasLength(2));
    },
  );

  for (final brokenEnumeration in [false, true]) {
    for (final source in ['legacy', 'community', 'account']) {
      test(
        '$source identity with broken enumeration=$brokenEnumeration forbids generation; explicit reuse preserves saved keys',
        () async {
          if (source == 'account') {
            await AccountIdentityStore(storage: storage).create(key.nsec);
          } else if (source == 'legacy') {
            storage.data['buzz_nsec'] = key.nsec;
          } else {
            storage.data['buzz_communities'] = jsonEncode([
              {'name': 'Saved', 'nsec': key.nsec, 'pubkey': key.public},
            ]);
            storage.data['buzz_nsec'] = other.nsec;
          }
          storage.failEnumeration = brokenEnumeration;
          final before = Map<String, String>.of(storage.data);
          final writesBefore = storage.writes;
          final inventory = await entry.choices();
          expect(inventory.complete, !brokenEnumeration);
          if (!brokenEnumeration || source != 'account') {
            expect(
              inventory.choices.map((choice) => choice.pubkey),
              contains(key.public),
            );
          }
          await expectLater(
            prepare(create: true),
            throwsA(
              brokenEnumeration
                  ? AccountIdentityFailure.unreadable
                  : AccountIdentityFailure.absent,
            ),
          );
          expect(storage.writes, writesBefore);
          expect(probes, isEmpty);
          expect(storage.data, before);
          final community = await prepare(savedPubkey: key.public);
          expect(community.nsec, key.nsec);
          expect(community.pubkey, key.public);
          expect(community.relayUrl, relay);
          for (final saved in before.entries) {
            expect(storage.data[saved.key], saved.value);
          }
          expect(probes, [community]);
        },
      );
    }
  }

  test(
    'account and community choices deduplicate the same saved identity',
    () async {
      await AccountIdentityStore(storage: storage).create(key.nsec);
      storage.data['buzz_nsec'] = key.nsec;
      expect((await entry.choices()).choices.map((choice) => choice.pubkey), [
        key.public,
      ]);
    },
  );

  test('unreadable account key never permits new generation', () async {
    storage.data[AccountIdentityStore.storageKey] = 'broken';
    await expectLater(
      prepare(create: true),
      throwsA(AccountIdentityFailure.unreadable),
    );
    expectUntouched();
  });

  test(
    'explicit replacement preserves refused key and retries the chosen key',
    () async {
      refuseProbe = true;
      await expectLater(prepare(nsec: key.nsec), throwsStateError);
      refuseProbe = false;
      final joined = await prepare(nsec: other.nsec);
      expect(joined.pubkey, other.public);
      final retained = AccountIdentityStore(
        storage: storage,
      ).forPairing(key.public);
      expect(await retained.withKeys((keys) async => keys.public), key.public);
      expect((await prepare(create: true)).pubkey, other.public);
      expect(probes, hasLength(3));
    },
  );

  test(
    'missing explicit saved identity never falls back to generation',
    () async {
      await expectLater(
        prepare(savedPubkey: key.public, create: true),
        throwsA(AccountIdentityFailure.absent),
      );
      expectUntouched();
    },
  );

  for (final boundary in ['before', 'read', 'write']) {
    test(
      'cancellation at $boundary fences probe and preserves any durable key',
      () async {
        if (boundary == 'before') current = false;
        if (boundary == 'read') storage.onRead = () => current = false;
        if (boundary == 'write') storage.onWrite = () => current = false;
        await expectLater(prepare(nsec: key.nsec), throwsStateError);
        expect(probes, isEmpty);
        expect(storage.writes, boundary == 'write' ? 1 : 0);
        if (boundary == 'before') expect(storage.reads, 0);
        if (boundary == 'write') {
          expect(storage.data.values, contains(key.nsec));
          current = true;
          storage.onWrite = null;
          expect((await prepare()).pubkey, key.public);
          expect(storage.writes, 1);
        } else {
          expect(storage.data, isEmpty);
        }
      },
    );
  }

  test('unconfirmed durable write cannot reach the relay probe', () async {
    storage.dropWrites = true;
    await expectLater(
      prepare(nsec: key.nsec),
      throwsA(AccountIdentityFailure.unreadable),
    );
    expect(probes, isEmpty);
    expect(storage.data, isEmpty);
  });
}

class _Storage extends Fake implements FlutterSecureStorage {
  final data = <String, String>{};
  int writes = 0;
  int reads = 0;
  bool dropWrites = false;
  bool failEnumeration = false;
  void Function()? onRead;
  void Function()? onWrite;

  @override
  Future<Map<String, String>> readAll({
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (failEnumeration) throw StateError('unrelated entry cannot decrypt');
    return Map.of(data);
  }

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    data.remove(key);
  }

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    reads++;
    onRead?.call();
    return data[key];
  }

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    writes++;
    if (!dropWrites && value != null) data[key] = value;
    onWrite?.call();
  }
}
