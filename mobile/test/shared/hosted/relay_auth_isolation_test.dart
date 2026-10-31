import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/organization_environment.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
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
  final organization = Uri.parse('https://block.example.com');
  const relayHost = 'buzz.block.example.com';
  const orgSessionKey = 'buzz_hosted_session_block_example_com_v1';
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));

  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  http.Response me() => http.Response(
    jsonEncode({'subject': 'member', 'expires_at': expiry.toIso8601String()}),
    200,
  );

  test(
    'a stalled organization issuer never delays a hosted relay',
    () async {
      final hostedRelay = RelayConfig(
        baseUrl: 'https://team.communities.buzz.xyz',
        nsec: keys.nsec,
      );
      final orgRelay = RelayConfig(
        baseUrl: 'https://$relayHost',
        nsec: keys.nsec,
      );
      for (final (origin, key) in [
        (organization, orgSessionKey),
        (
          BuilderlabAccountClient.origin,
          'buzz_hosted_session_app_builderlab_xyz_v1',
        ),
      ]) {
        await const FlutterSecureStorage().write(
          key: key,
          value: jsonEncode({
            'origin': origin.toString(),
            'credential': 'secret',
            'subject': 'member',
            'expires_at': expiry.toIso8601String(),
          }),
        );
      }
      final stalled = Completer<http.Response>();
      BuilderlabAccountClient client({required bool stall}) =>
          BuilderlabAccountClient(
            accountOrigin: stall ? organization : null,
            httpClient: MockClient((request) async {
              return switch (request.url.path) {
                '/api/goose/v1/auth/me' => me(),
                '/api/goose/v1/buzz/nostr-identities/current' => http.Response(
                  jsonEncode({
                    'identity': {'pubkey_hex': keys.public},
                  }),
                  200,
                ),
                '/api/goose/v1/buzz/identity/assertions' =>
                  stall
                      ? stalled.future
                      : http.Response(
                          jsonEncode({
                            'assertion': 'hosted-token',
                            'nostr_pubkey': keys.public,
                            'expires_at':
                                DateTime.now().millisecondsSinceEpoch ~/ 1000 +
                                120,
                          }),
                          200,
                        ),
                _ => http.Response('{}', 404),
              };
            }),
          );
      final auth = HostedRelayAuth(
        client(stall: false),
        organization: client(stall: true),
        organizationRelay: (relay) =>
            OrganizationEnvironment.matches(relay, [relayHost]),
        enabled: true,
      );
      final orgHeaders = auth.headers(orgRelay);
      await pumpEventQueue();
      expect(
        await auth.headers(hostedRelay).timeout(const Duration(seconds: 2)),
        {'Nostr-Federated-Identity': 'Bearer hosted-token'},
      );
      stalled.complete(http.Response('{}', 503));
      await expectLater(orgHeaders, throwsA(isA<AccountFailure>()));
      auth.dispose();
    },
    // Hosted relays are protected only in builds with authentication domains.
    skip: !HostedEnvironment.enabled,
  );
}
