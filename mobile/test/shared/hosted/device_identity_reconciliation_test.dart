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
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
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

  test(
    'unclaimed account offers a new identity beside saved keys and keeps them',
    () async {
      await secure.write(
        key: 'buzz_communities',
        value: jsonEncode([record(first, 'Community')]),
      );
      await AccountIdentityStore(subject: 'older').create(second.nsec);
      final before = await secure.readAll();
      final pending = await service.resolve();
      expect(pending.state, HostedIdentityState.needsSelection);
      expect(pending.canCreate, isTrue);
      final created = await service.resolve(
        createNew: true,
        offered: pending.choices.map((item) => item.pubkey).toSet(),
      );
      expect(created.state, HostedIdentityState.needsBinding);
      expect(created.pubkey, isNot(anyOf(first.public, second.public)));
      expect((await store.read())!.pubkey, created.pubkey);
      for (final entry in before.entries) {
        expect(await secure.read(key: entry.key), entry.value);
      }
      expect(account.calls.every((path) => path.endsWith('/status')), isTrue);
    },
  );

  test('creation refuses when only some device keys were offered', () async {
    await secure.write(key: 'buzz_nsec', value: first.nsec);
    await secure.write(
      key: 'buzz_communities',
      value: jsonEncode([record(second, 'Community')]),
    );
    final result = await service.resolve(
      createNew: true,
      offered: {first.public},
    );
    expect(result.state, HostedIdentityState.needsAuthoritativeAbsence);
    expect(await store.read(), isNull);
  });

  test(
    'creation never passes over a device key the user was not shown',
    () async {
      await secure.write(key: 'buzz_nsec', value: first.nsec);
      final result = await service.resolve(createNew: true);
      expect(result.state, HostedIdentityState.needsAuthoritativeAbsence);
      expect(await store.read(), isNull);
    },
  );

  test(
    'bound or backed-up accounts never offer or create a new identity',
    () async {
      await secure.write(key: 'buzz_nsec', value: first.nsec);
      account
        ..backups = [second.public]
        ..recovery = {
          'backups': [],
          'failed_backup_ids': ['pending'],
        };
      expect((await service.resolve()).canCreate, isFalse);
      account
        ..backups = []
        ..bound = first.public;
      final bound = await service.resolve(
        createNew: true,
        offered: {first.public},
      );
      expect(bound.canCreate, isFalse);
      expect(bound.pubkey, first.public);
    },
  );

  group('release after a rejected binding', () {
    Future<void> chosen({bool device = true, bool otherSlot = false}) async {
      if (device) await secure.write(key: 'buzz_nsec', value: first.nsec);
      if (otherSlot) {
        await AccountIdentityStore(subject: 'older').create(first.nsec);
      }
      await store.create(first.nsec);
    }

    for (final copy in ['device', 'other slot']) {
      test(
        'releases only a copy kept in the $copy, then offers creation',
        () async {
          await chosen(device: copy == 'device', otherSlot: copy != 'device');
          final before = await secure.readAll();
          expect(
            await service.releaseRejected(first.public, subject: 'fixture'),
            isTrue,
          );
          expect(await store.read(), isNull);
          for (final entry in before.entries) {
            final slot =
                '${AccountIdentityStore.storageKey}_subject_'
                '${base64Url.encode(utf8.encode('fixture'))}';
            if (entry.key == slot) continue;
            expect(await secure.read(key: entry.key), entry.value);
          }
          final choice = await service.resolve();
          expect(choice.state, HostedIdentityState.needsSelection);
          expect(choice.canCreate, isTrue);
          expect(choice.choices.map((item) => item.pubkey), [first.public]);
          // A retry after the slot is already gone converges.
          expect(
            await service.releaseRejected(first.public, subject: 'fixture'),
            isTrue,
          );
        },
      );
    }

    test('keeps the sole copy of a key', () async {
      await chosen(device: false);
      expect(
        await service.releaseRejected(first.public, subject: 'fixture'),
        isFalse,
      );
      expect((await store.read())!.pubkey, first.public);
    });

    test('keeps the slot once the account has a binding or backup', () async {
      await chosen();
      account.bound = second.public;
      expect(
        await service.releaseRejected(first.public, subject: 'fixture'),
        isFalse,
      );
      account
        ..bound = null
        ..backups = [second.public];
      expect(
        await service.releaseRejected(first.public, subject: 'fixture'),
        isFalse,
      );
      expect((await store.read())!.pubkey, first.public);
    });

    test('keeps the slot when the account reports a binding', () async {
      await chosen();
      final split = _BoundAccount(second.public);
      final release = HostedIdentityService(account: split, store: store);
      expect(
        await release.releaseRejected(first.public, subject: 'fixture'),
        isFalse,
      );
      expect((await store.read())!.pubkey, first.public);
      split.dispose();
    });

    test('the store never releases a receipt-bearing slot', () async {
      await chosen();
      await store.confirmBackup(
        BackupReceipt(
          backend: BuilderlabAccountClient.origin.origin,
          subject: 'fixture',
          pubkey: first.public,
          backupId: 'backup',
        ),
      );
      expect(
        await store.releaseUnconfirmed(
          first.public,
          heldElsewhere: (_) async => true,
          checkCurrent: () {},
        ),
        isFalse,
      );
      expect((await store.read())!.backup, isNotNull);
    });

    test('keeps a receipt-bearing or different key', () async {
      await chosen();
      expect(
        await service.releaseRejected(second.public, subject: 'fixture'),
        isFalse,
      );
      await store.confirmBackup(
        BackupReceipt(
          backend: BuilderlabAccountClient.origin.origin,
          subject: 'fixture',
          pubkey: first.public,
          backupId: 'backup',
        ),
      );
      expect(
        await service.releaseRejected(first.public, subject: 'fixture'),
        isFalse,
      );
      expect((await store.read())!.backup, isNotNull);
    });

    test('a failed delete keeps the slot and a retry converges', () async {
      final storage = _InventoryStorage()..failDelete = true;
      final flaky = AccountIdentityStore(storage: storage, subject: 'fixture');
      final release = HostedIdentityService(account: account, store: flaky);
      await chosen();
      await expectLater(
        release.releaseRejected(first.public, subject: 'fixture'),
        throwsA(AccountIdentityFailure.unreadable),
      );
      expect((await store.read())!.pubkey, first.public);
      storage.failDelete = false;
      expect(
        await release.releaseRejected(first.public, subject: 'fixture'),
        isTrue,
      );
      expect(await store.read(), isNull);
      expect(await secure.read(key: 'buzz_nsec'), first.nsec);
    });

    test('another account or an unusable recheck releases nothing', () async {
      await chosen();
      await expectLater(
        service.releaseRejected(first.public, subject: 'other'),
        throwsA(AccountFailure.differentAccount),
      );
      final broken = _BrokenStatusAccount();
      await expectLater(
        HostedIdentityService(
          account: broken,
          store: store,
        ).releaseRejected(first.public, subject: 'fixture'),
        throwsA(AccountFailure.invalidResponse),
      );
      broken.dispose();
      expect((await store.read())!.pubkey, first.public);
    });

    test('a cancelled attempt releases nothing', () async {
      await chosen();
      await expectLater(
        service.releaseRejected(
          first.public,
          subject: 'fixture',
          isCurrent: () => false,
        ),
        throwsA(AccountFailure.unavailable),
      );
      expect((await store.read())!.pubkey, first.public);
    });
  });

  group('release boundaries', () {
    late _SwitchingAccount switching;
    late _HookedDevice device;
    late HostedIdentityService release;
    var cancelled = false;
    setUp(() async {
      cancelled = false;
      switching = _SwitchingAccount();
      device = _HookedDevice();
      release = HostedIdentityService(
        account: switching,
        store: store,
        deviceIdentities: device,
      );
      await secure.write(key: 'buzz_nsec', value: first.nsec);
      await store.create(first.nsec);
      await AccountIdentityStore(subject: 'other').create(second.nsec);
    });
    tearDown(() => switching.dispose());

    Future<void> kept() async {
      expect((await store.read())!.pubkey, first.public);
      expect(
        (await AccountIdentityStore(subject: 'other').read())!.pubkey,
        second.public,
      );
      expect(await secure.read(key: 'buzz_nsec'), first.nsec);
    }

    for (final point in ['current', 'status', 'holds']) {
      test('an account switch at $point releases nothing', () async {
        switching.switchAt = point;
        device.beforeHolds = () async => switching.reach('holds');
        await expectLater(
          release.releaseRejected(first.public, subject: 'fixture'),
          throwsA(AccountFailure.differentAccount),
        );
        await kept();
      });
    }

    test(
      'cancelling between the recheck and the delete releases nothing',
      () async {
        device.beforeHolds = () async => cancelled = true;
        await expectLater(
          release.releaseRejected(
            first.public,
            subject: 'fixture',
            isCurrent: () => !cancelled,
          ),
          throwsA(AccountFailure.unavailable),
        );
        await kept();
      },
    );

    test(
      'an unreadable device inventory releases nothing, then converges',
      () async {
        device.beforeHolds = () async => throw StateError('keychain locked');
        await expectLater(
          release.releaseRejected(first.public, subject: 'fixture'),
          throwsStateError,
        );
        await kept();
        device.beforeHolds = null;
        expect(
          await release.releaseRejected(first.public, subject: 'fixture'),
          isTrue,
        );
        expect(await store.read(), isNull);
        expect(await secure.read(key: 'buzz_nsec'), first.nsec);
      },
    );
  });

  test('cancelled resolution cannot adopt or generate', () async {
    await secure.write(key: 'buzz_nsec', value: first.nsec);
    await expectLater(
      service.resolve(isCurrent: () => false),
      throwsA(AccountFailure.unavailable),
    );
    expect(await store.read(), isNull);
  });
}

/// Moves to another account when release reaches [switchAt].
class _SwitchingAccount extends InventoryAccount {
  String subject = 'fixture';
  String? switchAt;
  void reach(String point) {
    if (point == switchAt) subject = 'other';
  }

  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: subject, expiresAt: DateTime.utc(2100));
  @override
  Future<Map<String, dynamic>> currentIdentity() async {
    reach('current');
    return super.currentIdentity();
  }

  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    if (path.endsWith('/status')) reach('status');
    return super.post(path, body, expectedSubject: expectedSubject);
  }
}

/// Runs [beforeHolds] as release checks the device for another copy.
class _HookedDevice extends DeviceIdentitySource {
  Future<void> Function()? beforeHolds;
  @override
  Future<bool> holds(String nsec) async {
    await beforeHolds?.call();
    return super.holds(nsec);
  }
}

/// Reports a binding through /current while /status still lists none.
class _BoundAccount extends InventoryAccount {
  _BoundAccount(this.current);
  final String current;
  @override
  Future<Map<String, dynamic>> currentIdentity() async => {
    'identity': {'pubkey_hex': current},
  };
}

/// Answers backup status without a usable backup list.
class _BrokenStatusAccount extends InventoryAccount {
  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async => path.endsWith('/status')
      ? {'current_nostr_pubkey': null, 'backup_nostr_pubkeys': 'none'}
      : super.post(path, body, expectedSubject: expectedSubject);
}

class _InventoryStorage extends FlutterSecureStorage {
  bool failEnumeration = false;
  bool failDelete = false;
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
    if (failDelete) throw StateError('keychain unavailable');
    return super.delete(key: key);
  }

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
