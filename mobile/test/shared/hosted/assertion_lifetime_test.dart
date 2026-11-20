import 'dart:convert';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'an assertion with no lifetime left is reissued, then reported',
    () async {
      final keys = nostr.Keys('0' * 63 + '1');
      final expiry = DateTime.now().toUtc().add(const Duration(days: 1));
      FlutterSecureStorage.setMockInitialValues({});
      await const FlutterSecureStorage().write(
        key: 'buzz_hosted_session_block_example_com_v1',
        value: jsonEncode({
          'origin': 'https://block.example.com',
          'credential': 'org-secret',
          'subject': 'member',
          'expires_at': expiry.toIso8601String(),
        }),
      );
      // Each 10 s assertion spends 20 s in flight.
      const iat = 1800000000;
      var elapsed = Duration.zero;
      var issued = 0;
      String segment(Object v) =>
          base64Url.encode(utf8.encode(jsonEncode(v))).replaceAll('=', '');
      http.Response json(Object body) => http.Response(jsonEncode(body), 200);
      final account = BuilderlabAccountClient(
        accountOrigin: Uri.parse('https://block.example.com'),
        httpClient: MockClient((request) async {
          if (request.url.path.endsWith('/identity/assertions')) {
            issued++;
            elapsed += const Duration(seconds: 20);
            final claims = {'iat': iat, 'exp': iat + 10};
            return json({
              'assertion': '${segment({})}.${segment(claims)}.c2ln',
              'nostr_pubkey': keys.public,
              'expires_at': iat + 10,
            });
          }
          return request.url.path.endsWith('/nostr-identities/current')
              ? json({
                  'identity': {'pubkey_hex': keys.public},
                })
              : json({
                  'subject': 'member',
                  'expires_at': expiry.toIso8601String(),
                });
        }),
      );
      final auth = HostedRelayAuth(
        BuilderlabAccountClient(httpClient: MockClient((_) async => fail(''))),
        organization: account,
        organizationRelay: (_) => true,
        enabled: true,
        now: () => DateTime.fromMillisecondsSinceEpoch((iat - 100) * 1000),
        elapsed: () => elapsed,
      );
      final relay = RelayConfig(
        baseUrl: 'https://buzz.block.example.com',
        nsec: keys.nsec,
      );
      await expectLater(
        auth.headers(relay),
        throwsA(AccountFailure.unavailable),
      );
      expect(issued, 3);
      expect(auth.cachedHeaders(relay), isNull);
    },
  );

  test(
    'an assertion that expired while the device slept is reissued',
    () async {
      final keys = nostr.Keys('0' * 63 + '1');
      final expiry = DateTime.now().toUtc().add(const Duration(days: 1));
      FlutterSecureStorage.setMockInitialValues({});
      await const FlutterSecureStorage().write(
        key: 'buzz_hosted_session_block_example_com_v1',
        value: jsonEncode({
          'origin': 'https://block.example.com',
          'credential': 'org-secret',
          'subject': 'member',
          'expires_at': expiry.toIso8601String(),
        }),
      );
      // Each assertion lives 300 s from the issuer's wall time; the phone
      // reads that plus [correction].
      var wall = 1800000000;
      var elapsed = Duration.zero;
      var correction = 0;
      var issued = 0;
      String segment(Object v) =>
          base64Url.encode(utf8.encode(jsonEncode(v))).replaceAll('=', '');
      http.Response json(Object body) => http.Response(jsonEncode(body), 200);
      final account = BuilderlabAccountClient(
        accountOrigin: Uri.parse('https://block.example.com'),
        httpClient: MockClient((request) async {
          if (request.url.path.endsWith('/identity/assertions')) {
            issued++;
            final claims = {'iat': wall, 'exp': wall + 300};
            return json({
              'assertion': '${segment({})}.${segment(claims)}.c2ln',
              'nostr_pubkey': keys.public,
              'expires_at': wall + 300,
            });
          }
          return request.url.path.endsWith('/nostr-identities/current')
              ? json({
                  'identity': {'pubkey_hex': keys.public},
                })
              : json({
                  'subject': 'member',
                  'expires_at': expiry.toIso8601String(),
                });
        }),
      );
      final auth = HostedRelayAuth(
        BuilderlabAccountClient(httpClient: MockClient((_) async => fail(''))),
        organization: account,
        organizationRelay: (_) => true,
        enabled: true,
        now: () =>
            DateTime.fromMillisecondsSinceEpoch((wall + correction) * 1000),
        elapsed: () => elapsed,
      );
      final relay = RelayConfig(
        baseUrl: 'https://buzz.block.example.com',
        nsec: keys.nsec,
      );
      final first = await auth.headers(relay);
      expect(issued, 1);
      expect(auth.cachedHeaders(relay), first);
      // Sleep: the wall clock moves on while the monotonic clock does not.
      wall += 600;
      elapsed += const Duration(seconds: 1);
      expect(auth.cachedHeaders(relay), isNull);
      await auth.headers(relay);
      expect(issued, 2);
      // 200 s later, the phone's clock is corrected 120 s forward. The token
      // has 100 s of server lifetime left, but the wall bound now retires it:
      // one re-mint, never a refusal.
      wall += 200;
      elapsed += const Duration(seconds: 200);
      correction = 120;
      expect(auth.cachedHeaders(relay), isNull);
      final reminted = await auth.headers(relay);
      expect(issued, 3);
      expect(auth.cachedHeaders(relay), reminted);
    },
  );
}
