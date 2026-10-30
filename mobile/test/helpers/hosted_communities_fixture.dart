import 'dart:convert';
import 'dart:async';

import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

class DiscoveryBrowser extends BrowserAuth {
  final callbacks = StreamController<BrowserAuthCode>.broadcast();
  Map<String, String>? startedIntent;
  @override
  Future<void> start(Map<String, String> intent) async {
    startedIntent = intent;
  }

  @override
  Stream<BrowserAuthCode> get codes => callbacks.stream;
  @override
  Future<void> dispose() async {
    await callbacks.close();
    await super.dispose();
  }

  @override
  Future<void> initialize() async {}
  @override
  Future<void> clearIntent() async {}
  @override
  Future<void> cancel() async {}
}

class WidgetDiscoveryAccount extends BuilderlabAccountClient {
  WidgetDiscoveryAccount(this.fixture);
  final HostedCommunitiesFixture fixture;
  @override
  BuilderlabAccount? get account => fixture.expired
      ? null
      : BuilderlabAccount(
          subject: fixture.subject,
          expiresAt: DateTime.now().add(const Duration(hours: 1)),
        );
  @override
  Future<BuilderlabAccount?> restore() async => account;
  @override
  Future<void> rememberCommunity(
    String relay, {
    required String expectedSubject,
  }) async {
    if (account?.subject != expectedSubject) throw AccountFailure.denied;
    fixture.remembered.add(relay);
  }

  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    if (path.endsWith('/current')) {
      if (fixture.rejectCurrent) throw AccountFailure.denied;
      return {
        'identity': {'pubkey_hex': fixture.bound},
      };
    }
    if (path.endsWith('/known-communities/list')) {
      return {'communities': fixture.rows};
    }
    throw AccountFailure.unavailable;
  }
}

class HostedCommunitiesFixture {
  final keys = nostr.Keys('1'.padLeft(64, '0'));
  AccountIdentityStore get store => AccountIdentityStore(subject: subject);
  final browser = DiscoveryBrowser();
  late BuilderlabAccountClient account;
  late final HostedFlow flow;
  final admitted = <String>[];
  String? openedCommunity;
  Future<void> Function()? beforeOpen;
  final remembered = <String>[];
  final requests = <String>[];
  List<Map<String, dynamic>> rows = [];
  String? bound;
  bool rejectList = false;
  bool rejectCurrent = false;
  bool expired = false;
  String subject = 'owner';
  Future<void> Function(String)? beforeAdmission;
  Future<void> Function()? beforeList;
  Uri? opened;

  Map<String, dynamic> row(String name) => {
    'community_url': 'wss://$name.communities.buzz.xyz',
  };

  Future<void> initialize({bool backup = true, bool widget = false}) async {
    bound = keys.public;
    final expiry = DateTime.now()
        .toUtc()
        .add(const Duration(hours: 1))
        .toIso8601String();
    account = BuilderlabAccountClient(
      httpClient: MockClient((request) async {
        final path = request.url.path;
        requests.add(path);
        if (path.endsWith('/exchange')) {
          return http.Response(
            jsonEncode({'session_credential': 'fixture', 'expires_at': expiry}),
            200,
          );
        }
        if (path.endsWith('/me')) {
          if (expired) return http.Response('{}', 401);
          return http.Response(
            jsonEncode({'subject': subject, 'expires_at': expiry}),
            200,
          );
        }
        if (path.endsWith('/current')) {
          return http.Response(
            jsonEncode({
              'identity': {'pubkey_hex': bound},
            }),
            200,
          );
        }
        if (path.endsWith('/known-communities/list')) {
          await beforeList?.call();
          if (rejectList) return http.Response('{}', 503);
          return http.Response(jsonEncode({'communities': rows}), 200);
        }
        if (path.endsWith('/known-communities/add')) {
          final body = jsonDecode(request.body) as Map<String, dynamic>;
          return http.Response(
            jsonEncode({
              'community': {'community_url': body['community_url']},
            }),
            200,
          );
        }
        return http.Response('{}', 503);
      }),
    );
    if (widget) {
      account.dispose();
      account = WidgetDiscoveryAccount(this);
    } else {
      await account.exchange(code: 'code', verifier: 'a' * 43);
    }
    await store.create(keys.nsec);
    if (backup) {
      await store.confirmBackup(
        BackupReceipt(
          backend: BuilderlabAccountClient.origin.toString(),
          subject: 'owner',
          pubkey: keys.public,
          backupId: 'fixture-backup',
        ),
      );
    }
    flow = HostedFlow(
      account: account,
      browser: browser,
      store: store,
      identity: HostedIdentityService(account: account, store: store),
      openWeb: (uri) async {
        opened = uri;
        return true;
      },
      openLandingCommunity: (link, subject, pubkey, current) async {
        await beforeOpen?.call();
        if (current()) openedCommunity = link.relayUrl;
      },
      enterCommunity: (link, subject, pubkey, current) async {
        final target = link as AddCommunityDeepLink;
        await beforeAdmission?.call(target.name);
        if (current()) admitted.add(target.name);
      },
    );
  }

  Future<void> dispose() async {
    flow.dispose();
    await browser.dispose();
    account.dispose();
  }
}
