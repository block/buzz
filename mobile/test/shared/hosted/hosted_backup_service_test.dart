import 'dart:convert';
import 'dart:io';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_backup_service.dart';
import 'package:buzz/shared/hosted/hpke_backup.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

class FakeAccount extends BuilderlabAccountClient {
  FakeAccount(this.envelope);
  final Map<String, dynamic> envelope;
  int enrollments = 0;
  final uploads = <String>[];
  bool fail = true;
  bool reject = false;
  bool rejectEnrollment = false;
  String subject = 'subject';
  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: subject, expiresAt: DateTime(2100));
  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    if (path.endsWith('/enrollment')) {
      enrollments++;
      if (rejectEnrollment) throw AccountFailure.rejected;
      return envelope;
    }
    uploads.add(jsonEncode(body));
    expect(
      await const FlutterSecureStorage().read(
        key: HostedBackupService.pendingKey(
          subject,
          envelope['nostr_pubkey'] as String,
        ),
      ),
      isNotNull,
    );
    if (reject) throw AccountFailure.rejected;
    if (fail) throw AccountFailure.unavailable;
    return {'stored': true, 'backup_id': envelope['backup_id']};
  }
}

class FakeHpke extends HpkeBackup {
  FakeHpke(this.envelope);
  final Map<String, dynamic> envelope;
  int calls = 0;
  @override
  Future<Map<String, dynamic>> seal({
    required Map<String, dynamic> enrollment,
    required String secretHex,
  }) async {
    calls++;
    return envelope;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late Map<String, dynamic> envelope;
  setUp(() {
    FlutterSecureStorage.setMockInitialValues({});
    final fixture =
        jsonDecode(
              File(
                '../desktop/src-tauri/src/testdata/hpke_nsec_backup_v1.json',
              ).readAsStringSync(),
            )
            as Map<String, dynamic>;
    envelope = Map<String, dynamic>.from(fixture['envelope'] as Map);
  });
  test(
    'uncertain upload replays persisted envelope after restart and receipt prevents further upload',
    () async {
      final identity = AccountIdentityStore();
      await identity.create(nostr.Keys('42' * 32).nsec);
      final account = FakeAccount(envelope);
      final hpke = FakeHpke(envelope);
      await expectLater(
        HostedBackupService(account, identity, hpke: hpke).backup(),
        throwsA(AccountFailure.unavailable),
      );
      expect((await identity.read())!.backup, isNull);
      account.fail = false;
      await HostedBackupService(
        account,
        AccountIdentityStore(),
        hpke: hpke,
      ).backup();
      expect(account.enrollments, 1);
      expect(hpke.calls, 1);
      expect(account.uploads, hasLength(2));
      expect(account.uploads.first, account.uploads.last);
      expect((await identity.read())!.backup!.backupId, envelope['backup_id']);
      await HostedBackupService(account, identity, hpke: hpke).backup();
      expect(account.uploads, hasLength(2));
    },
  );
  test('unreadable pending ciphertext blocks enrollment and upload', () async {
    final identity = AccountIdentityStore();
    await identity.create(nostr.Keys('42' * 32).nsec);
    await const FlutterSecureStorage().write(
      key: HostedBackupService.pendingKey(
        'subject',
        envelope['nostr_pubkey'] as String,
      ),
      value: 'invalid',
    );
    final account = FakeAccount(envelope);
    await expectLater(
      HostedBackupService(account, identity).backup(),
      throwsA(HostedBackupFailure.storage),
    );
    expect(account.enrollments, 0);
    expect(account.uploads, isEmpty);
  });
  test(
    'definitive rejection permits only one re-enrollment per attempt',
    () async {
      final identity = AccountIdentityStore();
      await identity.create(nostr.Keys('42' * 32).nsec);
      final account = FakeAccount(envelope)..reject = true;
      await expectLater(
        HostedBackupService(
          account,
          identity,
          hpke: FakeHpke(envelope),
        ).backup(),
        throwsA(HostedBackupFailure.rejected),
      );
      expect(account.enrollments, 2);
      expect(account.uploads, hasLength(2));
      expect((await identity.read())!.backup, isNull);
    },
  );
  test('enrollment rejection is terminal without upload', () async {
    final identity = AccountIdentityStore();
    await identity.create(nostr.Keys('42' * 32).nsec);
    final account = FakeAccount(envelope)..rejectEnrollment = true;
    await expectLater(
      HostedBackupService(account, identity).backup(),
      throwsA(HostedBackupFailure.unavailable),
    );
    expect(account.enrollments, 1);
    expect(account.uploads, isEmpty);
  });
  test(
    'another subject pending envelope neither blocks nor gets replaced',
    () async {
      final identity = AccountIdentityStore();
      await identity.create(nostr.Keys('42' * 32).nsec);
      final oldKey = HostedBackupService.pendingKey(
        'other',
        envelope['nostr_pubkey'] as String,
      );
      await const FlutterSecureStorage().write(
        key: oldKey,
        value: 'old pending',
      );
      final account = FakeAccount(envelope)..fail = false;
      await HostedBackupService(
        account,
        identity,
        hpke: FakeHpke(envelope),
      ).backup();
      expect(
        await const FlutterSecureStorage().read(key: oldKey),
        'old pending',
      );
      expect(account.enrollments, 1);
    },
  );
}
