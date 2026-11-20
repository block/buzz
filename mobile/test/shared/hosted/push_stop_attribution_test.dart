import 'dart:convert';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/builderlab_build.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../diagnostics/recording_diagnostics.dart';

// A push stop for a removed community runs after sign-out, with the session
// that just ended. A reply it cannot understand is reported only while reports
// name the community's key or no one.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final keys = nostr.Keys.generate();
  final organization = Uri.parse('https://account.example.com');
  const relayHost = 'buzz.example.com';
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));
  const current = '/api/goose/v1/buzz/nostr-identities/current';
  const assertions = '/api/goose/v1/buzz/identity/assertions';

  // Which request gets a reply that is not a JSON object.
  for (final garbled in [current, assertions]) {
    for (final member in NamedMember.values) {
      test('a garbled reply to ${garbled.split('/').last} is '
          '${member.reported ? '' : 'not '}reported while reports name '
          '${member.label}', () async {
        FlutterSecureStorage.setMockInitialValues({});
        await const FlutterSecureStorage().write(
          key: 'buzz_hosted_session_account_example_com_v1',
          value: jsonEncode({
            'origin': organization.toString(),
            'credential': 'secret',
            'subject': 'member',
            'expires_at': expiry.toIso8601String(),
          }),
        );
        final diagnostics = RecordingDiagnostics();
        final account = BuilderlabAccountClient(
          accountOrigin: organization,
          diagnostics: diagnostics,
          httpClient: MockClient((request) async {
            if (request.url.path == garbled) return http.Response('[]', 200);
            return switch (request.url.path) {
              '/api/goose/v1/auth/me' => http.Response(
                jsonEncode({
                  'subject': 'member',
                  'expires_at': expiry.toIso8601String(),
                }),
                200,
              ),
              current => http.Response(
                jsonEncode({
                  'identity': {'pubkey_hex': keys.public},
                }),
                200,
              ),
              _ => http.Response('{}', 404),
            };
          }),
        );
        final auth = HostedRelayAuth(
          account,
          organization: account,
          organizationRelay: (relay) =>
              BuilderlabBuild.matchesRelay(relay, [relayHost]),
          enabled: true,
          discover: (_) async => true,
        );
        addTearDown(auth.dispose);
        // The session the push stop runs on is the one that just ended.
        expect(await account.restore(), isNotNull);
        await account.clearLocalSession();
        diagnostics.setMember(member.pubkey(keys.public));

        final headers = await auth.headers(
          RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec),
          background: true,
        );

        // A push stop never fails on an assertion it cannot get: the relay
        // decides.
        expect(headers, isEmpty);
        expect(diagnostics.operations, [
          if (member.reported) DiagnosticOperation.accountResponse,
        ]);
      });
    }
  }
}
