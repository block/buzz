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
  const current = '/api/goose/v1/buzz/nostr-identities/current';
  const assertions = '/api/goose/v1/buzz/identity/assertions';

  // A reply the app cannot read, for a community's key that may not be the
  // member reports name: a removed community's push stop, say, while another
  // community is active. Its report would name the wrong member.
  for (final (label, garbled, signedOut, background) in [
    ('an assertion', assertions, false, false),
    ('the current identity, signed in,', current, false, true),
    ('the current identity, signed out,', current, true, true),
    ('an assertion, signed out,', assertions, true, true),
  ]) {
    for (final (member, pubkey, reported) in [
      ('another member', nostr.Keys.generate().public, false),
      ('the signer', keys.public, true),
      ('no member', null, true),
    ]) {
      test('an unreadable reply to $label with $member set is '
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
        final diagnostics = _RecordingDiagnostics()..setMember(pubkey);
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
        // A push stop after sign-out runs on the session that just ended.
        if (signedOut) {
          expect(await account.restore(), isNotNull);
          await account.clearLocalSession();
          diagnostics.setMember(pubkey);
        }

        final headers = auth.headers(
          RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec),
          background: background,
        );

        // A push stop never fails on an assertion it cannot get: the relay
        // decides.
        if (background) {
          expect(await headers, isEmpty);
        } else {
          await expectLater(headers, throwsA(isA<AccountFailure>()));
        }
        expect(diagnostics.operations, [
          if (reported) DiagnosticOperation.accountResponse,
        ]);
      });
    }
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
