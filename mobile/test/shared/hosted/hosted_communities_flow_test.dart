import 'dart:async';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  Future<HostedCommunitiesFixture> fixture({bool backup = true}) async {
    final result = HostedCommunitiesFixture();
    await result.initialize(backup: backup);
    addTearDown(result.dispose);
    return result;
  }

  for (final failure in ['binding', 'backup']) {
    test(
      '$failure proof failure blocks discovery and preserves saved identity',
      () async {
        final f = await fixture(backup: failure != 'backup');
        if (failure == 'binding') f.bound = '2' * 64;
        final before = await const FlutterSecureStorage().readAll();
        await f.flow.signInAccount();
        expect(f.flow.stage, HostedFlowStage.blocked);
        if (failure == 'binding') {
          expect(f.flow.problem, HostedFlowProblem.identityMismatch);
        }
        expect(
          f.requests.any((path) => path.endsWith('/communities/joined')),
          isFalse,
        );
        expect(await const FlutterSecureStorage().readAll(), before);
        expect(f.admitted, isEmpty);
      },
    );
  }

  test('malformed list targets cannot authorize admission', () async {
    final f = await fixture();
    f.rows = [
      {...f.row('one'), 'normalized_host': 'wss://user@evil.example/path'},
    ];
    await f.flow.signInAccount();
    expect(f.flow.problem, HostedFlowProblem.invalidResponse);
    expect(f.admitted, isEmpty);
  });
  test('expired A batch is discarded after browser login as B', () async {
    final f = await fixture();
    await f.flow.initialize();
    f.rows = [f.row('a-one'), f.row('a-two')];
    f.beforeAdmission = (name) async {
      if (name == 'a-two') throw AccountFailure.unavailable;
    };
    await f.flow.signInAccount();
    expect(f.admitted, ['a-one']);
    f.expired = true;
    await f.flow.retry();
    expect(f.flow.stage, HostedFlowStage.signIn);
    expect(f.browser.startedIntent, {'type': 'account'});
    f.expired = false;
    f.subject = 'other-owner';
    f.rows = [f.row('b-one')];
    await f.store.create(f.keys.nsec);
    await f.store.confirmBackup(
      BackupReceipt(
        backend: BuilderlabAccountClient.origin.toString(),
        subject: f.subject,
        pubkey: f.keys.public,
        backupId: 'other-backup',
      ),
    );
    final completed = Completer<void>();
    f.flow.addListener(() {
      if (f.flow.stage == HostedFlowStage.complete && !completed.isCompleted) {
        completed.complete();
      }
    });
    f.browser.callbacks.add(
      BrowserAuthCode('other-code', 'a' * 43, {'type': 'account'}),
    );
    await completed.future.timeout(const Duration(seconds: 2));
    expect(f.admitted, ['a-one', 'b-one']);
  });
}
