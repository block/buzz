import 'dart:io';

import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_admission.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../diagnostics/recording_diagnostics.dart';

// The sockets that check a community before it is joined belong to a member
// who is not active yet, so what they report goes out only while reports name
// that member or no one.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('the self-hosted entry probe', () {
    // The test binding refuses real connections; the probe uses a loopback one.
    setUp(() {
      final overrides = HttpOverrides.current;
      HttpOverrides.global = null;
      addTearDown(() => HttpOverrides.global = overrides);
    });

    Community community(HttpServer server, nostr.Keys keys) => Community.create(
      name: 'Example',
      relayUrl: 'ws://127.0.0.1:${server.port}',
      pubkey: keys.public,
      nsec: keys.nsec,
    );

    for (final member in NamedMember.values) {
      test('reports a frame it cannot decode while reports name '
          '${member.label}', () async {
        final server = await loopbackRelay(first: ['not json']);
        final keys = nostr.Keys.generate();
        final diagnostics = RecordingDiagnostics()
          ..setMember(member.pubkey(keys.public));

        await SelfHostedEntry.probeRelay(
          community(server, keys),
          const {},
          diagnostics,
        );

        expect(diagnostics.operations, [
          if (member.reported) DiagnosticOperation.relayFrame,
        ]);
      });

      test('built by the app reports as the probe does while reports name '
          '${member.label}', () async {
        final server = await loopbackRelay(first: ['not json']);
        final keys = nostr.Keys.generate();
        final diagnostics = RecordingDiagnostics()
          ..setMember(member.pubkey(keys.public));
        final container = ProviderContainer(
          overrides: [
            diagnosticReporterProvider.overrideWithValue(diagnostics),
          ],
        );
        addTearDown(container.dispose);

        await container.read(relayProbeProvider)(
          community(server, keys),
          const {},
        );

        expect(diagnostics.operations, [
          if (member.reported) DiagnosticOperation.relayFrame,
        ]);
      });
    }

    test(
      'a relay that is not a Buzz relay is reported once and refused',
      () async {
        final server = await loopbackRelay(
          first: ['<html>Welcome to nginx</html>'],
          challenge: false,
          closeAfterFirst: true,
        );
        final keys = nostr.Keys.generate();
        final diagnostics = RecordingDiagnostics();

        await expectLater(
          SelfHostedEntry.probeRelay(
            community(server, keys),
            const {},
            diagnostics,
          ),
          throwsA(isA<StateError>()),
        );

        expect(diagnostics.operations, [DiagnosticOperation.relayFrame]);
      },
    );
  });

  group('hosted admission', () {
    final keys = nostr.Keys.generate();
    const link = AddCommunityDeepLink(
      relayUrl: 'wss://example.communities.buzz.xyz',
      name: 'Example',
    );

    for (final member in NamedMember.values) {
      test('reports a frame it cannot decode while reports name '
          '${member.label}', () async {
        FlutterSecureStorage.setMockInitialValues({});
        final server = await loopbackRelay(first: ['not json']);
        final account = _Account(keys.public);
        addTearDown(account.dispose);
        final store = AccountIdentityStore(subject: 'owner');
        await store.create(keys.nsec);
        await store.confirmBackup(
          BackupReceipt(
            backend: BuilderlabAccountClient.origin.origin,
            subject: 'owner',
            pubkey: keys.public,
            backupId: 'backup',
          ),
        );
        final diagnostics = RecordingDiagnostics()
          ..setMember(member.pubkey(keys.public));
        final admission = HostedAdmission(
          account: account,
          relayAuth: HostedRelayAuth(account, enabled: false),
          store: store,
          diagnostics: diagnostics,
        );
        addTearDown(admission.dispose);

        // The admission connects to its own relay address; the test relay
        // answers there instead.
        final community = await HttpOverrides.runWithHttpOverrides(
          () => admission.prepare(link, 'owner', keys.public),
          _Loopback(server.port),
        );

        expect(community.pubkey, keys.public);
        expect(diagnostics.operations, [
          if (member.reported) DiagnosticOperation.relayFrame,
        ]);
      });
    }
  });
}

class _Account extends BuilderlabAccountClient {
  _Account(this.bound);

  final String bound;

  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: 'owner', expiresAt: DateTime.utc(2100));

  @override
  Future<Map<String, dynamic>> currentIdentity() async => {
    'identity': {'pubkey_hex': bound},
  };
}

/// Sends every connection to the loopback relay on [port], whatever address
/// it was made for.
class _Loopback extends HttpOverrides {
  _Loopback(this.port);

  final int port;

  @override
  HttpClient createHttpClient(SecurityContext? context) =>
      _LoopbackClient(super.createHttpClient(context), port);
}

class _LoopbackClient extends Fake implements HttpClient {
  _LoopbackClient(this._inner, this._port);

  final HttpClient _inner;
  final int _port;

  @override
  String? get userAgent => _inner.userAgent;

  @override
  set userAgent(String? value) => _inner.userAgent = value;

  @override
  Future<HttpClientRequest> openUrl(String method, Uri url) => _inner.openUrl(
    method,
    url.replace(scheme: 'http', host: '127.0.0.1', port: _port),
  );

  @override
  void close({bool force = false}) => _inner.close(force: force);
}
