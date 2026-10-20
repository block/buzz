import 'dart:convert';

import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

class InventoryAccount extends BuilderlabAccountClient {
  String? bound;
  List<String> backups = [];
  Map<String, dynamic> recovery = {};
  final calls = <String>[];
  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: 'fixture', expiresAt: DateTime.utc(2100));
  @override
  Future<BuilderlabAccount?> restore() async => account;
  @override
  Future<Map<String, dynamic>> currentIdentity() async => {
    if (bound != null) 'identity': {'pubkey_hex': bound},
  };
  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body,
  ) async {
    calls.add(path);
    if (path.endsWith('/status')) {
      return {'current_nostr_pubkey': bound, 'backup_nostr_pubkeys': backups};
    }
    if (path.endsWith('/recovery')) return recovery;
    throw AccountFailure.denied;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final first = nostr.Keys('1'.padLeft(64, '0'));
  final second = nostr.Keys('2'.padLeft(64, '0'));
  const secure = FlutterSecureStorage();
  late InventoryAccount account;
  late AccountIdentityStore store;
  late HostedIdentityService service;
  Map<String, dynamic> record(nostr.Keys key, String name) => {
    'nsec': key.nsec,
    'pubkey': key.public,
    'name': name,
  };
  setUp(() {
    FlutterSecureStorage.setMockInitialValues({});
    account = InventoryAccount();
    store = AccountIdentityStore(subject: 'fixture');
    service = HostedIdentityService(
      account: account,
      store: store,
      deviceIdentities: DeviceIdentitySource(),
    );
  });
  tearDown(() => account.dispose());

  for (final scenario in [
    (
      name: 'unbound',
      bound: false,
      backups: <String>[],
      state: HostedIdentityState.needsBinding,
    ),
    (
      name: 'bound with matching backup',
      bound: true,
      backups: [first.public],
      state: HostedIdentityState.needsBackup,
    ),
    (
      name: 'bound with other backup',
      bound: true,
      backups: [first.public, second.public],
      state: HostedIdentityState.needsBackup,
    ),
  ]) {
    test('device key reconciliation: ${scenario.name}', () async {
      final raw = jsonEncode([record(first, 'Old community')]);
      await secure.write(key: 'buzz_workspaces', value: raw);
      account.bound = scenario.bound ? first.public : null;
      account.backups = scenario.backups;
      if (!scenario.bound) {
        expect(
          (await service.resolve()).state,
          HostedIdentityState.needsSelection,
        );
        expect(await store.read(), isNull);
      }
      final result = await service.resolve(selectedPubkey: first.public);
      expect(result.state, scenario.state);
      expect(result.pubkey, first.public);
      expect((await store.read())?.pubkey, first.public);
      expect(await secure.read(key: 'buzz_workspaces'), raw);
      expect(await secure.read(key: 'buzz_communities'), isNull);
      expect(account.calls.every((path) => path.endsWith('/status')), isTrue);
    });
  }

  test(
    'multiple current and hidden legacy keys require choice; no writes before choice',
    () async {
      await secure.write(
        key: 'buzz_communities',
        value: jsonEncode([record(first, 'Current')]),
      );
      await secure.write(key: 'buzz_nsec', value: second.nsec);
      final before = await secure.readAll();
      final pending = await service.resolve();
      expect(pending.state, HostedIdentityState.needsSelection);
      expect(pending.choices.map((item) => item.pubkey), [
        first.public,
        second.public,
      ]);
      expect(await secure.readAll(), before);
      final selected = await service.resolve(selectedPubkey: second.public);
      expect(selected.pubkey, second.public);
      for (final entry in before.entries) {
        expect(await secure.read(key: entry.key), entry.value);
      }
      expect(account.calls.every((path) => path.endsWith('/status')), isTrue);
      expect(
        (await service.resolve(selectedPubkey: first.public)).pubkey,
        second.public,
      );
    },
  );

  test('duplicate device identities offer one explicit reuse choice', () async {
    FlutterSecureStorage.setMockInitialValues({
      'buzz_nsec': first.nsec,
      'buzz_communities': jsonEncode([record(first, 'Same identity')]),
    });
    expect((await service.resolve()).choices.map((item) => item.pubkey), [
      first.public,
    ]);
    expect(
      (await service.resolve(selectedPubkey: first.public)).pubkey,
      first.public,
    );
  });

  test(
    'corrupt, incomplete and mismatched legacy records never authorize generation',
    () async {
      for (final data in [
        {'buzz_workspaces': '{broken'},
        {'buzz_nsec': ''},
        {
          'buzz_communities': jsonEncode([
            {'nsec': first.nsec, 'pubkey': second.public},
          ]),
        },
      ]) {
        FlutterSecureStorage.setMockInitialValues(data);
        await expectLater(
          service.resolve(),
          throwsA(AccountIdentityFailure.unreadable),
        );
        expect(await secure.readAll(), data);
      }
      // Another account's scoped key must remain a choice, even when an
      // unrelated Android entry prevents bulk enumeration. Never mint a key
      // from that incomplete inventory; retry after storage recovers.
      FlutterSecureStorage.setMockInitialValues({});
      final storage = _InventoryStorage();
      store = AccountIdentityStore(storage: storage, subject: 'fixture');
      await store.forSubject('other-account').create(first.nsec);
      service = HostedIdentityService(account: account, store: store);
      final before = await secure.readAll();
      expect(
        (await service.resolve()).state,
        HostedIdentityState.needsSelection,
      );
      storage.failEnumeration = true;
      await expectLater(
        service.resolve(),
        throwsA(AccountIdentityFailure.unreadable),
      );
      expect(await secure.readAll(), before);
      expect(await store.read(), isNull);
      storage.failEnumeration = false;
      expect((await service.resolve()).choices.single.pubkey, first.public);
      expect(
        (await service.resolve(selectedPubkey: first.public)).pubkey,
        first.public,
      );
      expect(
        (await store.forSubject('other-account').read())!.pubkey,
        first.public,
      );
    },
  );

  test(
    'token-only legacy records do not prevent recovering the same cloud identity',
    () async {
      final raw = jsonEncode([
        {'pubkey': first.public, 'name': 'Token-only community'},
      ]);
      await secure.write(key: 'buzz_communities', value: raw);
      await secure.write(key: 'buzz_pubkey', value: first.public);
      account.bound = first.public;
      account.backups = [first.public];
      account.recovery = {
        'backups': [
          {
            'backup_id': 'fixture-backup',
            'nostr_pubkey': first.public,
            'secret_key': base64UrlEncode([
              ...List.filled(31, 0),
              1,
            ]).replaceAll('=', ''),
          },
        ],
        'failed_backup_ids': [],
      };
      final result = await service.resolve();
      expect(result.state, HostedIdentityState.ready);
      expect(result.pubkey, first.public);
      expect(account.calls, [
        '/v1/buzz/nsec-backups/status',
        '/v1/buzz/nsec-backups/recovery',
      ]);
      expect(await secure.read(key: 'buzz_communities'), raw);
      expect(await secure.read(key: 'buzz_pubkey'), first.public);
    },
  );

  test(
    'inventory labels are deduplicated and empty names get a fallback',
    () async {
      final raw = jsonEncode([record(first, 'Team'), record(second, '  ')]);
      await secure.write(key: 'buzz_communities', value: raw);
      await secure.write(key: 'buzz_workspaces', value: raw);
      final choices = await DeviceIdentitySource().choices();
      expect(choices.map((choice) => choice.label), [
        'Team',
        'Saved community',
      ]);
    },
  );

  test('cancelled resolution cannot adopt or generate', () async {
    await secure.write(key: 'buzz_nsec', value: first.nsec);
    await expectLater(
      service.resolve(isCurrent: () => false),
      throwsA(AccountFailure.unavailable),
    );
    expect(await store.read(), isNull);
  });
}

class _InventoryStorage extends FlutterSecureStorage {
  bool failEnumeration = false;
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
    return super.readAll();
  }
}
