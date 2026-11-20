import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

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

    for (final member in _Member.values) {
      test('a frame it cannot decode is ${member.reported ? '' : 'not '}'
          'reported while reports name ${member.label}', () async {
        final server = await loopbackRelay(first: ['not json']);
        final keys = nostr.Keys.generate();
        final diagnostics = _RecordingDiagnostics()
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

      test('a frame it cannot decode, with the probe built by the app, is '
          '${member.reported ? '' : 'not '}reported while reports name '
          '${member.label}', () async {
        final server = await loopbackRelay(first: ['not json']);
        final keys = nostr.Keys.generate();
        final diagnostics = _RecordingDiagnostics()
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
        final diagnostics = _RecordingDiagnostics();

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

  // The flow the app builds hands its reporter to the relay check that admits a
  // community, whose socket belongs to a member who is not active yet.
  group('the hosted flow built by the app', () {
    final keys = nostr.Keys.generate();
    const link = AddCommunityDeepLink(
      relayUrl: 'wss://example.communities.buzz.xyz',
      name: 'Example',
    );

    for (final member in _Member.values) {
      test('a frame it cannot decode is ${member.reported ? '' : 'not '}'
          'reported while reports name ${member.label}', () async {
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
        final diagnostics = _RecordingDiagnostics()
          ..setMember(member.pubkey(keys.public));
        final relayAuth = HostedRelayAuth(account, enabled: false);
        addTearDown(relayAuth.dispose);
        final container = ProviderContainer(
          overrides: [
            diagnosticReporterProvider.overrideWithValue(diagnostics),
            builderlabAccountProvider.overrideWithValue(account),
            hostedRelayAuthProvider.overrideWithValue(relayAuth),
            authProvider.overrideWith(_Auth.new),
            activeCommunityProvider.overrideWith((ref) async => null),
          ],
        );
        addTearDown(container.dispose);

        // The admission connects to its own relay address; the test relay
        // answers there instead.
        await HttpOverrides.runWithHttpOverrides(
          () => container
              .read(hostedFlowProvider)
              .enterCommunity(link, 'owner', keys.public, () => true),
          _Loopback(server.port),
        );

        expect(diagnostics.operations, [
          if (member.reported) DiagnosticOperation.relayFrame,
        ]);
      });
    }
  });
}

/// Takes the community a flow admits without saving it.
class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() => Completer<AuthState>().future;

  @override
  Future<void> authenticateWithCommunity(
    Community community, {
    bool Function()? shouldContinue,
    bool activate = true,
  }) async {}
}

class _Account extends BuilderlabAccountClient {
  _Account(this.bound);

  final String bound;

  @override
  BuilderlabAccount get account =>
      BuilderlabAccount(subject: 'owner', expiresAt: DateTime.utc(2100));

  @override
  Future<Map<String, dynamic>> currentIdentity({String? signer}) async => {
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

/// A reporter that records the operation of each report. The member is
/// whatever the test last named.
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

/// Who a report names while a request signed by one key is in flight.
enum _Member {
  other('another member'),
  signer('the signer'),
  none('no one');

  const _Member(this.label);
  final String label;

  /// Whether a report from work signed by [signer] goes out.
  bool get reported => this != other;

  String? pubkey(String signer) => switch (this) {
    other => 'e' * 64,
    _Member.signer => signer,
    none => null,
  };
}

/// A loopback relay that sends [first] as raw frames (text or binary) as soon as a
/// client connects, then the NIP-42 challenge, and accepts the AUTH it gets
/// back. [onFrame] sees every other frame the client sends, with the socket.
Future<HttpServer> loopbackRelay({
  List<Object> first = const [],
  void Function(WebSocket socket, List<dynamic> frame)? onFrame,
  bool challenge = true,
  bool closeAfterFirst = false,
}) async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  addTearDown(() => server.close(force: true));
  server.listen((request) async {
    final ws = await WebSocketTransformer.upgrade(request);
    for (final frame in first) {
      ws.add(frame);
    }
    if (closeAfterFirst) {
      await ws.close();
      return;
    }
    ws.listen((raw) {
      final frame = jsonDecode(raw as String) as List<dynamic>;
      if (frame.first == 'AUTH') {
        final id = (frame[1] as Map<String, dynamic>)['id'];
        ws.add(jsonEncode(['OK', id, true, '']));
      } else {
        onFrame?.call(ws, frame);
      }
    });
    if (challenge) ws.add(jsonEncode(['AUTH', 'challenge']));
  });
  return server;
}
