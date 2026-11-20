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

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final keys = nostr.Keys.generate();
  final organization = Uri.parse('https://block.example.com');
  const relayHost = 'buzz.block.example.com';
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));

  // An assertion for a removed community, say, while another community is
  // active: its report would name the wrong member.
  for (final (label, member, reported) in [
    ('another member', nostr.Keys.generate().public, false),
    ('the signer', keys.public, true),
    ('no member', null, true),
  ]) {
    test('an unreadable assertion with $label set is '
        '${reported ? '' : 'not '}reported', () async {
      FlutterSecureStorage.setMockInitialValues({});
      await const FlutterSecureStorage().write(
        key: 'buzz_hosted_session_block_example_com_v1',
        value: jsonEncode({
          'origin': organization.toString(),
          'credential': 'secret',
          'subject': 'member',
          'expires_at': expiry.toIso8601String(),
        }),
      );
      final diagnostics = _RecordingDiagnostics()..setMember(member);
      BuilderlabAccountClient client({Uri? origin}) => BuilderlabAccountClient(
        accountOrigin: origin,
        diagnostics: diagnostics,
        httpClient: MockClient(
          (request) async => switch (request.url.path) {
            '/api/goose/v1/auth/me' => http.Response(
              jsonEncode({
                'subject': 'member',
                'expires_at': expiry.toIso8601String(),
              }),
              200,
            ),
            '/api/goose/v1/buzz/nostr-identities/current' => http.Response(
              jsonEncode({
                'identity': {'pubkey_hex': keys.public},
              }),
              200,
            ),
            '/api/goose/v1/buzz/identity/assertions' => http.Response(
              '[]',
              200,
            ),
            _ => http.Response('{}', 404),
          },
        ),
      );
      final auth = HostedRelayAuth(
        client(),
        organization: client(origin: organization),
        organizationRelay: (relay) =>
            BuilderlabBuild.matchesRelay(relay, [relayHost]),
        enabled: true,
        discover: (_) async => true,
      );
      addTearDown(auth.dispose);

      await expectLater(
        auth.headers(
          RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec),
        ),
        throwsA(isA<AccountFailure>()),
      );

      expect(diagnostics.operations, [
        if (reported) DiagnosticOperation.accountResponse,
      ]);
    });
  }
}

class _RecordingDiagnostics extends NoopDiagnosticReporter {
  final operations = <DiagnosticOperation>[];
  String? _member;

  @override
  String? get member => _member;

  @override
  void setMember(String? pubkey) => _member = pubkey;

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) => operations.add(operation);
}
