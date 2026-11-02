import 'dart:async';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/hosted/account_identity_store.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final key = nostr.Keys(
    '0000000000000000000000000000000000000000000000000000000000000001',
  );
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  for (final simulated in [false, true]) {
    test(
      'recovery retains scoped receipt; simulated=$simulated cannot authorize',
      () async {
        final receipt = BackupReceipt(
          backend: 'https://app.builderlab.xyz',
          subject: 'account-a',
          pubkey: key.public,
          backupId: simulated ? 'debug-simulated-backup:old' : 'backup-1',
        );
        final store = AccountIdentityStore();
        expect(await store.read(), isNull);
        await store.create(key.nsec, recoveredBackup: receipt);
        final restored = (await AccountIdentityStore().read())!;
        expect(
          restored.hasBackupFor(
            backend: receipt.backend,
            subject: receipt.subject,
          ),
          !simulated,
        );
        expect(
          restored.hasBackupFor(backend: receipt.backend, subject: 'account-b'),
          isFalse,
        );
        expect(await store.withKeys((keys) async => keys.public), key.public);
        expect((await store.create(key.nsec)).pubkey, key.public);
        // Existing receipt migrates only to its owner; another account has
        // an independent create-only slot and cannot replace the first key.
        final owner = store.forSubject('account-a');
        final otherAccount = store.forSubject('account-b');
        expect((await owner.read())!.pubkey, key.public);
        expect(await otherAccount.read(), isNull);
        final otherKey = nostr.Keys('2'.padLeft(64, '0'));
        await otherAccount.create(otherKey.nsec);
        expect((await owner.read())!.pubkey, key.public);
        expect((await store.read())!.pubkey, key.public);
        expect((await otherAccount.read())!.pubkey, otherKey.public);
        await store.forPairing(key.public).create(key.nsec);
        await store.forPairing(otherKey.public).create(otherKey.nsec);
        expect((await store.identities()).map((item) => item.pubkey).toSet(), {
          key.public,
          otherKey.public,
        });
      },
    );
  }

  test(
    'unrelated enumeration failure preserves known-slot access and errors',
    () async {
      final storage = EnumerationFailureStorage();
      final legacy = AccountIdentityStore(storage: storage);
      await legacy.create(key.nsec);
      final scoped = legacy.forSubject('b');
      final other = nostr.Keys('2'.padLeft(64, '0'));
      await scoped.create(other.nsec);
      await expectLater(
        scoped.identities(),
        throwsA(AccountIdentityFailure.unreadable),
      );
      expect((await scoped.read())!.pubkey, other.public);
      expect(
        await scoped.withSavedIdentity(key.public, (value) async => value),
        key.nsec,
      );
      await storage.write(
        key: AccountIdentityStore.storageKey,
        value: 'broken',
      );
      await expectLater(
        scoped.identities(),
        throwsA(AccountIdentityFailure.unreadable),
      );
    },
  );

  test(
    'unreadable storage never allows creation or touches legacy keys',
    () async {
      FlutterSecureStorage.setMockInitialValues({
        AccountIdentityStore.storageKey: 'broken',
        'buzz_communities': 'legacy',
      });
      final store = AccountIdentityStore();
      await expectLater(
        store.read(),
        throwsA(AccountIdentityFailure.unreadable),
      );
      await expectLater(
        store.create(key.nsec),
        throwsA(AccountIdentityFailure.unreadable),
      );
      expect(
        await const FlutterSecureStorage().read(key: 'buzz_communities'),
        'legacy',
      );
    },
  );

  test('invalid candidate and wrong-key receipt never write', () async {
    final store = AccountIdentityStore();
    await expectLater(
      store.create('not-a-key'),
      throwsA(AccountIdentityFailure.invalidKey),
    );
    final other = nostr.Keys(
      '0000000000000000000000000000000000000000000000000000000000000002',
    );
    await expectLater(
      store.create(
        key.nsec,
        recoveredBackup: BackupReceipt(
          backend: 'https://app.builderlab.xyz',
          subject: 'a',
          pubkey: other.public,
          backupId: 'b',
        ),
      ),
      throwsA(AccountIdentityFailure.mismatch),
    );
    expect(await store.read(), isNull);
    await store.create(key.nsec);
    await expectLater(
      store.confirmBackup(
        BackupReceipt(
          backend: 'https://app.builderlab.xyz',
          subject: 'a',
          pubkey: other.public,
          backupId: 'b',
        ),
      ),
      throwsA(AccountIdentityFailure.mismatch),
    );
    expect((await store.read())!.backup, isNull);
  });

  for (final generate in [false, true]) {
    test(
      'uncertain ${generate ? "generated" : "recovered"} write retries exact identity once',
      () async {
        final storage = UncertainStorage();
        final store = AccountIdentityStore(storage: storage);
        var authorizations = 0;
        final receipt = BackupReceipt(
          backend: 'https://app.builderlab.xyz',
          subject: 'a',
          pubkey: key.public,
          backupId: 'one',
        );
        Future<AccountIdentitySnapshot?> write() => generate
            ? store.generateIfAbsent(
                authorize: () async {
                  authorizations++;
                  return true;
                },
              )
            : store.create(key.nsec, recoveredBackup: receipt);
        await expectLater(write(), throwsA(AccountIdentityFailure.unreadable));
        final persisted = (await store.read())!;
        final retried = (await write())!;
        expect(retried.pubkey, persisted.pubkey);
        expect(authorizations, generate ? 1 : 0);
        if (!generate) {
          expect(retried.backup?.backupId, 'one');
          final other = nostr.Keys('2'.padLeft(64, '0'));
          await expectLater(
            store.create(other.nsec),
            throwsA(AccountIdentityFailure.occupied),
          );
          await expectLater(
            store.create(
              key.nsec,
              recoveredBackup: BackupReceipt(
                backend: receipt.backend,
                subject: 'b',
                pubkey: key.public,
                backupId: 'two',
              ),
            ),
            throwsA(AccountIdentityFailure.mismatch),
          );
        }
        expect(storage.writes, 1);
      },
    );
  }

  test(
    'late durable write blocks second instance create until settled',
    () async {
      final storage = DelayedStorage();
      final first = AccountIdentityStore(storage: storage).create(key.nsec);
      await storage.started.future;
      final second = AccountIdentityStore(storage: storage).create(key.nsec);
      final retried = second;
      storage.release.complete();
      await first;
      await retried;
      expect(storage.writes, 1);
    },
  );
}

class DelayedStorage extends FlutterSecureStorage {
  final started = Completer<void>();
  final release = Completer<void>();
  int writes = 0;
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
    started.complete();
    await release.future;
    return super.write(key: key, value: value);
  }
}

class UncertainStorage extends FlutterSecureStorage {
  int writes = 0;
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
    await super.write(key: key, value: value);
    throw StateError('plugin response lost');
  }
}

class EnumerationFailureStorage extends FlutterSecureStorage {
  @override
  Future<Map<String, String>> readAll({
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => throw StateError('unrelated entry cannot decrypt');
}
