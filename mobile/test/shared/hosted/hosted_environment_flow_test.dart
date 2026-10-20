import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_admission.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

class _Account extends BuilderlabAccountClient {
  _Account(this.pubkey);
  final String pubkey;
  var reads = 0;
  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: 'owner', expiresAt: DateTime.utc(2100));
  @override
  Future<BuilderlabAccount?> restore() async {
    reads++;
    return account;
  }

  @override
  Future<Map<String, dynamic>> currentIdentity() async {
    reads++;
    return {
      'identity': {'pubkey_hex': pubkey},
    };
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));
  test(
    'real flow and admission use only the configured community realm',
    () async {
      final staging =
          HostedEnvironment.origin.host == 'app.builderlabstaging.xyz';
      final matching = staging
          ? 'team.communities.public.builderlabstaging.xyz'
          : 'team.communities.buzz.xyz';
      final opposite = staging
          ? 'team.communities.buzz.xyz'
          : 'team.communities.public.builderlabstaging.xyz';
      final keys = nostr.Keys('1'.padLeft(64, '0'));
      final account = _Account(keys.public);
      final store = AccountIdentityStore(subject: 'owner');
      await store.create(keys.nsec);
      await store.confirmBackup(
        BackupReceipt(
          backend: HostedEnvironment.origin.origin,
          subject: 'owner',
          pubkey: keys.public,
          backupId: 'receipt',
        ),
      );
      final requests = <http.Request>[];
      final admission = HostedAdmission(
        account: account,
        relayAuth: HostedRelayAuth(account, enabled: false),
        store: store,
        probe: (_) async {},
        client: MockClient((request) async {
          requests.add(request);
          return http.Response('{}', 200);
        }),
      );
      final browser = BrowserAuth(
        openBrowser: (_) async => fail('Saved account must not open browser'),
      );
      var entered = 0;
      final flow = HostedFlow(
        account: account,
        browser: browser,
        store: store,
        identity: HostedIdentityService(account: account, store: store),
        enterCommunity: (link, subject, pubkey, isCurrent) async {
          await admission.prepare(link, subject, pubkey, isCurrent: isCurrent);
          entered++;
        },
      );
      addTearDown(() async {
        flow.dispose();
        await browser.dispose();
        account.dispose();
        admission.dispose();
      });
      await flow.start(
        InviteDeepLink(relayUrl: 'wss://$opposite', code: 'test'),
      );
      expect(flow.problem, HostedFlowProblem.unsupportedEnvironment);
      expect(account.reads, 0);
      expect(requests, isEmpty);
      // Admission independently enforces the boundary, even without the UI/flow.
      await expectLater(
        admission.prepare(
          InviteDeepLink(relayUrl: 'wss://$opposite', code: 'test'),
          'owner',
          keys.public,
        ),
        throwsA(AccountFailure.denied),
      );
      expect(account.reads, 0);
      await flow.start(
        InviteDeepLink(relayUrl: 'wss://$matching', code: 'test'),
      );
      expect(flow.stage, HostedFlowStage.complete);
      expect(entered, 1);
      expect(requests.map((request) => request.url.host), [matching, matching]);
      expect(requests.map((request) => request.method), ['GET', 'POST']);
      expect(requests.map((request) => request.url.path), [
        '/api/join-policy',
        '/api/invites/claim',
      ]);
    },
  );
}
