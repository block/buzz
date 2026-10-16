import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/debug_backup.dart';
import 'package:buzz/shared/hosted/hosted_backup_service.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

class NoBackupNetwork extends BuilderlabAccountClient {
  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: 'test-account', expiresAt: DateTime(2100));

  @override
  Future<Map<String, dynamic>> post(String path, Map<String, dynamic> body) =>
      throw StateError('Simulation must not contact backup API');
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('simulated receipt only authorizes the explicit staging debug mode', () {
    final snapshot = AccountIdentitySnapshot(
      'a' * 64,
      BackupReceipt(
        backend: HostedEnvironment.origin.origin,
        subject: 'test-account',
        pubkey: 'a' * 64,
        backupId: '${DebugBackup.receiptPrefix}test',
      ),
    );
    expect(
      snapshot.hasBackupFor(
        backend: HostedEnvironment.origin.origin,
        subject: 'test-account',
      ),
      DebugBackup.enabled,
    );
  });

  test(
    'fake backup preserves key and makes no backup requests',
    () async {
      FlutterSecureStorage.setMockInitialValues({});
      final store = AccountIdentityStore();
      final key = nostr.Keys('42' * 32);
      await store.create(key.nsec);
      await HostedBackupService(NoBackupNetwork(), store).backup();
      final snapshot = (await store.read())!;
      expect(snapshot.pubkey, key.public);
      expect(snapshot.backup!.backupId, startsWith(DebugBackup.receiptPrefix));
      expect(
        snapshot.hasBackupFor(
          backend: HostedEnvironment.origin.origin,
          subject: 'test-account',
        ),
        isTrue,
      );
    },
    skip: !DebugBackup.enabled,
  );
}
