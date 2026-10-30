import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'dart:async';
import 'hosted_http_fixture.dart';
import 'package:buzz/shared/hosted/hosted_join_policy.dart';
import 'dart:convert';

import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_admission.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final keys = nostr.Keys(
    '0000000000000000000000000000000000000000000000000000000000000001',
  );
  const invite = InviteDeepLink(
    relayUrl: 'wss://example.communities.buzz.xyz',
    code: 'invite',
    policyReceipt: 'policy',
  );
  late _Account account;
  late AccountIdentityStore store;
  late List<http.Request> claims;
  late HostedAdmission admission;
  setUp(() {
    FlutterSecureStorage.setMockInitialValues({});
    account = _Account(keys.public);
    store = AccountIdentityStore(subject: 'owner');
    claims = [];
    admission = HostedAdmission(
      account: account,
      relayAuth: HostedRelayAuth(account, enabled: false),
      store: store,
      probe: (_) async {},
      client: MockClient((request) async {
        if (request.method == 'POST') claims.add(request);
        return http.Response('{}', 200);
      }),
    );
  });
  tearDown(() {
    admission.dispose();
    account.dispose();
  });
  Future<void> backup() async => store.confirmBackup(
    BackupReceipt(
      backend: BuilderlabAccountClient.origin.origin,
      subject: 'owner',
      pubkey: keys.public,
      backupId: 'backup',
    ),
  );

  test('no claim without scoped backup or current matching binding', () async {
    await store.create(keys.nsec);
    await expectLater(
      admission.prepare(invite, 'owner', keys.public),
      throwsA(AccountFailure.denied),
    );
    expect(account.bindingReads, 0);
    expect(claims, isEmpty);
    await backup();
    account.bound = 'a' * 64;
    await expectLater(
      admission.prepare(invite, 'owner', keys.public),
      throwsA(AccountFailure.denied),
    );
    expect(account.bindingReads, 1);
    expect(claims, isEmpty);
  });

  test('claim signature and returned identity use resolved key', () async {
    await store.create(keys.nsec);
    await backup();
    final community = await admission.prepare(invite, 'owner', keys.public);
    expect(claims, hasLength(1));
    final request = claims.single;
    final auth = request.headers['Authorization']!;
    expect(auth.startsWith('Nostr '), isTrue);
    final event = nostr.Event.fromJson(
      utf8.decode(base64.decode(auth.substring(6))),
      verify: true,
    );
    expect(event.pubkey, keys.public);
    expect(event.kind, 27235);
    expect(
      event.tags.map((tag) => tag.toList()).toList(),
      contains(
        equals(['u', 'https://example.communities.buzz.xyz/api/invites/claim']),
      ),
    );
    expect(
      event.tags.map((tag) => tag.toList()).toList(),
      contains(equals(['method', 'POST'])),
    );
    expect(jsonDecode(request.body), {'code': 'invite'});
    expect(community.pubkey, keys.public);
    expect(community.nsec, keys.nsec);
    expect(request.followRedirects, isFalse);
  });

  // Wire limits stay focused; consent/claim ordering is covered by the journey.
  for (final (name, response, status, failure) in [
    ('redirect', '{}', 302, AccountFailure.denied),
    ('oversized policy', ' ' * 65537, 200, AccountFailure.invalidResponse),
    ('invalid JSON', 'not JSON', 200, AccountFailure.invalidResponse),
    ('wrong shape', '[]', 200, AccountFailure.invalidResponse),
  ]) {
    test('$name cannot reach a claim', () async {
      await store.create(keys.nsec);
      await backup();
      admission.dispose();
      admission = HostedAdmission(
        account: account,
        relayAuth: HostedRelayAuth(account, enabled: false),
        store: store,
        probe: (_) async {},
        client: MockClient((request) async {
          expect(request.method, 'GET');
          expect(request.followRedirects, isFalse);
          expect(
            request.headers.containsKey('X-BB-Session-Credential'),
            isFalse,
          );
          return http.Response(response, status);
        }),
      );
      await expectLater(
        admission.prepare(invite, 'owner', keys.public),
        throwsA(failure),
      );
    });
  }

  testWidgets('total policy deadline cancels body and permits policy retry', (
    tester,
  ) async {
    final transport = TrickleClient((_) => http.Response('{}', 200));
    Future<String?> policy() => freshJoinPolicyReceipt(
      client: transport,
      origin: Uri.parse(invite.relayUrl),
      code: invite.code,
      confirm: (_) async =>
          fail('incomplete policy must never request consent'),
      checkCurrent: () {},
    );
    Object? failure;
    unawaited(
      policy().then<void>(
        (_) => fail('trickling policy must not complete'),
        onError: (Object error) {
          failure = error;
        },
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(seconds: 20));
    await tester.pump(const Duration(seconds: 5));
    expect(failure, isNull);
    await tester.pump(const Duration(seconds: 5));
    expect(failure, isA<TimeoutException>());
    expect(transport.aborted, isTrue);
    expect(transport.cancelled, isTrue);
    transport.slow = false;
    final retry = policy();
    await tester.pump();
    expect(await retry, isNull);
  });

  test('cancellation during binding prevents external claim', () async {
    await store.create(keys.nsec);
    await backup();
    var current = true;
    account.onBinding = () => current = false;
    await expectLater(
      admission.prepare(invite, 'owner', keys.public, isCurrent: () => current),
      throwsA(AccountFailure.unavailable),
    );
    expect(claims, isEmpty);
  });

  test('cancelled or rejected relay probe never returns community', () async {
    await store.create(keys.nsec);
    await backup();
    final pending = Completer<void>();
    final started = Completer<void>();
    var current = true;
    final delayed = HostedAdmission(
      account: account,
      relayAuth: HostedRelayAuth(account, enabled: false),
      store: store,
      probe: (_) {
        started.complete();
        return pending.future;
      },
    );
    const link = AddCommunityDeepLink(
      relayUrl: 'wss://example.communities.buzz.xyz',
      name: 'Wes',
    );
    final result = delayed.prepare(
      link,
      'owner',
      keys.public,
      isCurrent: () => current,
    );
    final expectation = expectLater(
      result,
      throwsA(AccountFailure.unavailable),
    );
    await started.future;
    current = false;
    pending.complete();
    await expectation;
    delayed.dispose();
    final rejected = HostedAdmission(
      account: account,
      relayAuth: HostedRelayAuth(account, enabled: false),
      store: store,
      probe: (_) async {
        throw AccountFailure.denied;
      },
    );
    await expectLater(
      rejected.prepare(link, 'owner', keys.public),
      throwsA(AccountFailure.denied),
    );
    rejected.dispose();
    expect(claims, isEmpty);
  });

  test('different account and untrusted host cannot claim', () async {
    await store.create(keys.nsec);
    await backup();
    await expectLater(
      admission.prepare(invite, 'other', keys.public),
      throwsA(AccountFailure.differentAccount),
    );
    await expectLater(
      admission.prepare(
        const InviteDeepLink(
          relayUrl: 'wss://communities.buzz.xyz.attacker.test',
          code: 'i',
        ),
        'owner',
        keys.public,
      ),
      throwsA(AccountFailure.denied),
    );
    expect(claims, isEmpty);
  });
}

class _Account extends BuilderlabAccountClient {
  _Account(this.bound);
  String bound;
  int bindingReads = 0;
  void Function()? onBinding;
  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: 'owner', expiresAt: DateTime.utc(2100));
  @override
  Future<Map<String, dynamic>> currentIdentity() async {
    bindingReads++;
    onBinding?.call();
    return {
      'identity': {'pubkey_hex': bound},
    };
  }
}
