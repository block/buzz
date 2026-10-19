import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'device_identity_reconciliation_test.dart' show InventoryAccount;

class _Browser extends BrowserAuth {
  Completer<void>? cancelling;
  int cleared = 0;
  @override
  Future<void> cancel() async => cancelling?.future;
  @override
  Future<void> clearIntent() async {
    cleared++;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final first = nostr.Keys('1'.padLeft(64, '0'));
  final second = nostr.Keys('2'.padLeft(64, '0'));
  late InventoryAccount account;
  late AccountIdentityStore store;
  late _Browser browser;
  late HostedFlow flow;
  const target = AddCommunityDeepLink(
    relayUrl: 'wss://team.communities.buzz.xyz',
    name: 'Team',
  );
  setUp(() {
    FlutterSecureStorage.setMockInitialValues({
      'buzz_communities': jsonEncode([
        {'name': 'First', 'pubkey': first.public, 'nsec': first.nsec},
        {'name': 'Second', 'pubkey': second.public, 'nsec': second.nsec},
      ]),
    });
    account = InventoryAccount();
    store = AccountIdentityStore(subject: 'fixture');
    browser = _Browser();
    flow = HostedFlow(
      account: account,
      browser: browser,
      store: store,
      identity: HostedIdentityService(
        account: account,
        store: store,
        deviceIdentities: DeviceIdentitySource(),
      ),
      enterCommunity: (_, _, _, _) async => fail('Must not bypass binding'),
    );
  });
  tearDown(() async {
    flow.dispose();
    await browser.dispose();
    account.dispose();
  });

  test(
    'unoffered choices cannot restart resolution or perform any remote work',
    () async {
      await flow.start(target);
      final calls = List.of(account.calls);
      final choices = List.of(flow.identityChoices);
      await flow.chooseIdentity('0' * 64);
      expect(flow.stage, HostedFlowStage.chooseIdentity);
      expect(account.calls, calls);
      expect(flow.identityChoices, choices);
      expect(await store.read(), isNull);
    },
  );

  test(
    'choices are retired on selection and on leaving; late tap cannot cancel leave',
    () async {
      await flow.start(target);
      browser.cancelling = Completer<void>();
      final calls = List.of(account.calls);
      final leaving = flow.leave();
      await flow.chooseIdentity(first.public);
      expect(account.calls, calls);
      expect(await store.read(), isNull);
      browser.cancelling!.complete();
      await leaving;
      expect(browser.cleared, 1);
      expect(flow.stage, HostedFlowStage.idle);
      expect(flow.identityChoices, isEmpty);
      await flow.start(target);
      final choosing = flow.chooseIdentity(second.public);
      expect(flow.identityChoices, isEmpty);
      await choosing;
      expect(flow.problem, HostedFlowProblem.denied);
      final completedCalls = List.of(account.calls);
      await flow.chooseIdentity(first.public);
      expect(account.calls, completedCalls);
      expect((await store.read())!.pubkey, second.public);
    },
  );
}
