import 'dart:convert';

import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  // Public test vectors only; no user signing material.
  final keys = [for (var i = 1; i <= 3; i++) nostr.Keys('$i'.padLeft(64, '0'))];
  Map<String, dynamic> backup(int index, {String? id}) => {
    'backup_id': id ?? 'backup-$index',
    'nostr_pubkey': keys[index].public,
    'secret_key': base64Url
        .encode([...List.filled(31, 0), index + 1])
        .replaceAll('=', ''),
  };

  late String? bound;
  late Map<String, dynamic> recovery;
  late List<String>? listedKeys;
  var statusCode = 200;
  late AccountIdentityStore store;
  late String communityJson;
  late int recoveryCalls;
  late int recoveryStatus;
  Future<void> Function()? duringRecovery;

  setUp(() {
    statusCode = 200;
    bound = null;
    listedKeys = [keys[0].public, keys[1].public];
    recovery = {
      'backups': [backup(0), backup(1)],
      'failed_backup_ids': [],
    };
    communityJson = jsonEncode([
      {'name': 'Existing', 'nsec': keys[2].nsec, 'pubkey': keys[2].public},
    ]);
    FlutterSecureStorage.setMockInitialValues({
      'buzz_communities': communityJson,
    });
    store = AccountIdentityStore(subject: 'a');
    recoveryCalls = 0;
    recoveryStatus = 200;
    duringRecovery = null;
  });

  Future<HostedIdentityService> service({
    bool includeDevice = false,
    Future<List<DeviceIdentityChoice>> Function()? inventory,
  }) async {
    final expiry = DateTime.now()
        .toUtc()
        .add(const Duration(hours: 1))
        .toIso8601String();
    final account = BuilderlabAccountClient(
      httpClient: MockClient((request) async {
        final path = request.url.path;
        if (path.endsWith('/exchange')) {
          return http.Response(
            jsonEncode({'session_credential': 'token', 'expires_at': expiry}),
            200,
          );
        }
        if (path.endsWith('/me')) {
          return http.Response(
            jsonEncode({'subject': 'a', 'expires_at': expiry}),
            200,
          );
        }
        if (path.endsWith('/current')) {
          return http.Response(
            jsonEncode({
              if (bound != null) 'identity': {'pubkey_hex': bound},
            }),
            200,
          );
        }
        if (path.endsWith('/status')) {
          return http.Response(
            jsonEncode({
              'current_nostr_pubkey': bound,
              'backup_nostr_pubkeys': ?listedKeys,
            }),
            statusCode,
          );
        }
        if (path.endsWith('/recovery')) {
          recoveryCalls++;
          await duringRecovery?.call();
          return http.Response(jsonEncode(recovery), recoveryStatus);
        }
        return http.Response('{}', 404);
      }),
    );
    await account.exchange(code: 'code', verifier: 'a' * 43);
    return HostedIdentityService(
      account: account,
      store: store,
      deviceIdentities: includeDevice
          ? DeviceIdentitySource()
          : _Inventory(inventory ?? () async => []),
    );
  }

  Future<void> expectCommunityPreserved() async {
    expect(
      await const FlutterSecureStorage().read(key: 'buzz_communities'),
      communityJson,
    );
  }

  test(
    'bound identity selects matching backup, persists receipt and resumes',
    () async {
      bound = keys[1].public;
      final resolver = await service();
      final result = await resolver.resolve(selectedPubkey: keys[0].public);
      expect(result.state, HostedIdentityState.ready);
      expect(result.pubkey, keys[1].public);
      final saved = (await store.read())!;
      expect(saved.backup!.backupId, 'backup-1');
      expect(
        saved.hasBackupFor(
          backend: BuilderlabAccountClient.origin.origin,
          subject: 'a',
        ),
        isTrue,
      );
      expect((await resolver.resolve()).pubkey, keys[1].public);
      expect(recoveryCalls, 1);
      await expectCommunityPreserved();
    },
  );

  test(
    'unbound distinct keys expose public choices and persist only selection',
    () async {
      final resolver = await service(includeDevice: true);
      final pending = await resolver.resolve();
      expect(pending.state, HostedIdentityState.needsSelection);
      expect(pending.choices.map((choice) => choice.pubkey), [
        keys[0].public,
        keys[1].public,
      ]);
      expect(pending.choices.every((choice) => choice.fromCloud), isTrue);
      expect(await store.read(), isNull);
      final chosen = await resolver.resolve(selectedPubkey: keys[1].public);
      expect(chosen.state, HostedIdentityState.needsBinding);
      expect(chosen.pubkey, keys[1].public);
      expect((await store.read())!.backup!.backupId, 'backup-1');
      await expectCommunityPreserved();
    },
  );

  test(
    'selected unbound recovery resumes binding after restart without choosing again',
    () async {
      final first = await service();
      final selected = await first.resolve(selectedPubkey: keys[1].public);
      expect(selected.state, HostedIdentityState.needsBinding);
      final receipt = (await store.read())!.backup!;
      final restarted = await service();
      final resumed = await restarted.resolve();
      expect(resumed.state, HostedIdentityState.needsBinding);
      expect(resumed.pubkey, keys[1].public);
      expect((await store.read())!.backup!.backupId, receipt.backupId);
      expect(recoveryCalls, 1);
      await expectCommunityPreserved();
    },
  );

  test(
    'local key without a current-account receipt cannot bypass distinct backup conflict',
    () async {
      await store.create(keys[1].nsec);
      final resolver = await service();
      expect((await resolver.resolve()).state, HostedIdentityState.mismatch);
      await expectLater(
        store.confirmBackup(
          BackupReceipt(
            backend: BuilderlabAccountClient.origin.origin,
            subject: 'another-account',
            pubkey: keys[1].public,
            backupId: 'other-account-backup',
          ),
        ),
        throwsA(AccountIdentityFailure.mismatch),
      );
      expect((await resolver.resolve()).state, HostedIdentityState.mismatch);
      expect(recoveryCalls, 0);
      expect((await store.read())!.pubkey, keys[1].public);
    },
  );

  test(
    'receipt does not bypass conflict when selected key is no longer listed',
    () async {
      final resolver = await service();
      await resolver.resolve(selectedPubkey: keys[1].public);
      listedKeys = [keys[0].public];
      expect((await resolver.resolve()).state, HostedIdentityState.mismatch);
      expect((await store.read())!.pubkey, keys[1].public);
      expect(recoveryCalls, 1);
    },
  );

  test(
    'unknown or removed selection asks again without selecting another key',
    () async {
      final resolver = await service();
      expect(
        (await resolver.resolve(selectedPubkey: keys[2].public)).state,
        HostedIdentityState.needsSelection,
      );
      recovery['backups'] = [backup(0)];
      listedKeys = [keys[0].public];
      expect(
        (await resolver.resolve(selectedPubkey: keys[1].public)).state,
        HostedIdentityState.needsSelection,
      );
      expect(await store.read(), isNull);
    },
  );

  test(
    'duplicate backups of one key do not create an ambiguous choice',
    () async {
      recovery['backups'] = [backup(0), backup(0, id: 'duplicate')];
      listedKeys = [keys[0].public];
      final result = await (await service()).resolve();
      expect(result.state, HostedIdentityState.needsBinding);
      expect(result.pubkey, keys[0].public);
    },
  );

  test('missing bound backup never accepts a different selected key', () async {
    bound = keys[2].public;
    expect(
      (await (await service()).resolve(selectedPubkey: keys[0].public)).state,
      HostedIdentityState.mismatch,
    );
    expect(await store.read(), isNull);
  });

  test('different device key still requires explicit cloud adoption', () async {
    bound = keys[1].public;
    final resolver = await service(includeDevice: true);
    final pending = await resolver.resolve();
    expect(pending.state, HostedIdentityState.needsSelection);
    expect(pending.choices.single.pubkey, keys[1].public);
    expect(
      (await resolver.resolve(selectedPubkey: keys[2].public)).state,
      HostedIdentityState.needsSelection,
    );
    expect(await store.read(), isNull);
    expect(
      (await resolver.resolve(selectedPubkey: keys[1].public)).state,
      HostedIdentityState.ready,
    );
    await expectCommunityPreserved();
  });

  for (final scenario in [
    'partial',
    'omitted',
    'unexpected',
    'malformed',
    'mismatched',
    'refused',
    'empty',
  ]) {
    test('recovery $scenario never writes or generates', () async {
      bound = keys[0].public;
      switch (scenario) {
        case 'partial':
          recovery['failed_backup_ids'] = ['unreadable'];
        case 'omitted':
          bound = null;
          recovery['backups'] = [backup(0)];
        case 'unexpected':
          bound = null;
          listedKeys = [keys[0].public];
        case 'malformed':
          (recovery['backups'] as List)[1]['secret_key'] = 'invalid';
        case 'mismatched':
          (recovery['backups'] as List)[1]['nostr_pubkey'] = keys[2].public;
        case 'refused':
          recoveryStatus = 403;
        case 'empty':
          bound = null;
          recovery['backups'] = [];
      }
      final resolver = await service();
      if (['partial', 'omitted', 'unexpected', 'empty'].contains(scenario)) {
        expect(
          (await resolver.resolve()).state,
          scenario == 'empty'
              ? HostedIdentityState.needsAuthoritativeAbsence
              : HostedIdentityState.recoveryBlocked,
        );
      } else {
        await expectLater(
          resolver.resolve(),
          throwsA(
            scenario == 'refused'
                ? isA<AccountFailure>()
                : AccountFailure.invalidResponse,
          ),
        );
      }
      expect(await store.read(), isNull);
      await expectCommunityPreserved();
    });
  }

  test(
    'occupied account slot is never replaced by a recovered choice',
    () async {
      final resolver = await service();
      duringRecovery = () async {
        await store.create(keys[2].nsec);
      };
      await expectLater(
        resolver.resolve(selectedPubkey: keys[1].public),
        throwsA(AccountIdentityFailure.occupied),
      );
      expect((await store.read())!.pubkey, keys[2].public);
      await expectCommunityPreserved();
    },
  );

  test('cancelled recovery cannot persist the selected identity', () async {
    var current = true;
    final resolver = await service();
    duringRecovery = () async {
      current = false;
    };
    await expectLater(
      resolver.resolve(
        selectedPubkey: keys[1].public,
        isCurrent: () => current,
      ),
      throwsA(AccountFailure.unavailable),
    );
    expect(await store.read(), isNull);
  });
  test(
    'saved identity skips recovery and conflicting binding never replaces it',
    () async {
      bound = keys[0].public;
      listedKeys = [keys[0].public];
      await store.create(keys[0].nsec);
      final resolver = await service();
      expect((await resolver.resolve()).state, HostedIdentityState.needsBackup);
      bound = keys[1].public;
      expect((await resolver.resolve()).state, HostedIdentityState.mismatch);
      expect(recoveryCalls, 0);
      expect((await store.read())!.pubkey, keys[0].public);
    },
  );

  test(
    'authoritative absence rechecks inventory before generating once',
    () async {
      listedKeys = [];
      var reads = 0;
      final blocked = await service(
        inventory: () async => ++reads == 1
            ? []
            : [DeviceIdentityChoice(keys[2].public, 'Newly saved')],
      );
      expect(
        (await blocked.resolve()).state,
        HostedIdentityState.needsAuthoritativeAbsence,
      );
      expect(await store.read(), isNull);
      final resolver = await service();
      final first = await resolver.resolve();
      expect(first.state, HostedIdentityState.needsBinding);
      expect((await resolver.resolve()).pubkey, first.pubkey);
      expect(recoveryCalls, 0);
    },
  );

  for (final scenario in ['403', '404', '500', 'missing', 'cancelled']) {
    test('unproven absence ($scenario) cannot generate', () async {
      var current = true;
      listedKeys = scenario == 'missing' ? null : [];
      statusCode = int.tryParse(scenario) ?? 200;
      final resolver = await service(
        inventory: () async {
          current = false;
          return [];
        },
      );
      await expectLater(
        resolver.resolve(isCurrent: () => current),
        throwsA(isA<AccountFailure>()),
      );
      expect(await store.read(), isNull);
      expect(recoveryCalls, 0);
      await expectCommunityPreserved();
    });
  }
}

// Substitute the external device inventory, not generation authorization.
class _Inventory extends DeviceIdentitySource {
  _Inventory(this.read);
  final Future<List<DeviceIdentityChoice>> Function() read;
  @override
  Future<List<DeviceIdentityChoice>> choices() => read();
}
