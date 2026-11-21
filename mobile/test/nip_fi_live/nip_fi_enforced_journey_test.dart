@Tags(['nip-fi-live'])
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/hosted/builderlab_build.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/native_media_headers.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import 'nip_fi_live_fixture.dart';

/// The mobile app's hosted-relay auth, run for real against a `buzz-relay`
/// that enforces NIP-FI. The relay's access log, read by
/// `mobile/scripts/nip_fi_live/oracle.py`, decides pass or fail: this test
/// only drives the journey and records a marker as each step begins.
///
/// Run it through `mobile/scripts/nip_fi_live/run.sh`, which starts the
/// fixture, compiles in the hosted build settings, and runs the oracle.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('hosted journey is admitted by a relay that enforces NIP-FI', () async {
    final fixture = NipFiFixture.load();
    // The test binding answers every request with a 400; this journey needs
    // the real sockets, aimed at the fixture.
    final previousOverrides = HttpOverrides.current;
    HttpOverrides.global = fixture.overrides();
    addTearDown(() => HttpOverrides.global = previousOverrides);
    FlutterSecureStorage.setMockInitialValues({});

    // Without these compile-time settings the app is not a hosted build and
    // would send no assertion at all, so a pass would prove nothing.
    expect(
      HostedRelayAuth.configured,
      isTrue,
      reason: 'compile with --dart-define=BUZZ_HOSTED_RELAY_AUTH=true',
    );
    expect(
      BuilderlabBuild.distribution,
      BuilderlabDistribution.public,
      reason: 'compile with --dart-define=BUZZ_DISTRIBUTION=public',
    );
    expect(BuilderlabBuild.origin, Uri.parse(fixture.accountOrigin));
    final relay = Uri.parse(fixture.relayOrigin);
    expect(HostedEnvironment.supportsRelay(relay), isTrue);
    expect(HostedEnvironment.requiresAuth(relay.host), isTrue);

    // Step 1, the negative control: with no evidence the relay refuses
    // `/query` and says it requires NIP-FI. This is what proves the later
    // steps ran against an enforcing relay.
    fixture.mark('control_start');
    final plain = http.Client();
    addTearDown(plain.close);
    final nip11 = jsonDecode(
      (await plain.get(
        relay.replace(path: '/'),
        headers: {'Accept': 'application/nostr+json'},
      )).body,
    );
    expect((nip11 as Map)['requires_nip_fi'], isTrue);
    final refused = await plain.post(
      relay.replace(path: '/query'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode([
        {
          'kinds': [39000],
        },
      ]),
    );
    expect(refused.statusCode, 401);
    fixture.mark('control_end');

    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixtureRelayConfig(fixture.relayOrigin, fixture.nsec),
        ),
        authProvider.overrideWith(_AuthenticatedAuth.new),
      ],
    );
    addTearDown(container.dispose);

    // The rest of the journey runs to its end even when a step fails, so the
    // relay's log shows every route that was refused, not only the first.
    // A failed step is recorded here and fails the test at the end.
    final problems = <String>[];
    Future<void> step(String name, Future<void> Function() body) async {
      fixture.mark(name);
      try {
        await body();
      } on Object catch (error) {
        problems.add('$name: $error');
      }
    }

    void expectStatus(String what, int status) {
      if (status != 200) problems.add('$what answered HTTP $status');
    }

    // Step 2: sign in through the real account client, connect the real
    // session, then query, upload and read media through the real clients.
    await step('sign_in', () async {
      await container
          .read(builderlabAccountProvider)
          .exchange(code: 'live-check-code', verifier: 'v' * 43);
    });

    await step('connect', () async {
      final connected = Completer<void>();
      final listener = container.listen(relaySessionProvider, (_, next) {
        if (next.status == SessionStatus.connected && !connected.isCompleted) {
          connected.complete();
        }
      }, fireImmediately: true);
      addTearDown(listener.close);
      await connected.future.timeout(
        const Duration(seconds: 30),
        onTimeout: () => throw StateError(
          'the relay session never connected '
          '(${container.read(relaySessionProvider).status})',
        ),
      );
    });
    final session = container.read(relaySessionProvider.notifier);

    Future<void> query() async {
      await session.queryRelay(const [
        NostrFilter(kinds: [39000]),
      ]);
    }

    await step('query', query);

    BlobDescriptor? blob;
    await step('upload', () async {
      blob = await container
          .read(mediaUploadServiceProvider)
          .uploadBytes(_png, mimeType: 'image/png');
    });

    final url = blob?.url;
    if (url == null) {
      problems.add('no uploaded media to read back');
    } else {
      await step('media', () async {
        final media = await container
            .read(mediaHttpClientProvider)
            .get(
              Uri.parse(url),
              headers: container
                  .read(mediaGetAuthServiceProvider)
                  .headersFor(url),
            );
        expectStatus('GET media', media.statusCode);
        if (media.statusCode == 200 && !listEquals(media.bodyBytes, _png)) {
          problems.add('media bytes differ from the upload');
        }
      });

      // Step 3: a native player or image loader takes headers up front and
      // fetches on its own, outside the app's HTTP client.
      await step('native_media', () async {
        final headers = container.read(nativeMediaHeadersProvider)(url);
        expectStatus('native media read', await _nativeGet(url, headers));
      });

      // Wait past the assertion's lifetime with the app in the background, as
      // when a player fetches while the app is not on screen. The relay
      // session pauses after its grace period, so nothing renews the cached
      // assertion: a player about to fetch has to ask for a fresh one. (A live
      // session would reconnect at lease expiry and refresh the cache, which
      // hides the stale-assertion path this step exists to catch.)
      session.onAppPaused();
      fixture.mark('expiry_wait');
      await Future<void>.delayed(
        Duration(seconds: fixture.assertionLifetimeSecs + 3),
      );
      await step('native_media_after_expiry', () async {
        final headers = await container.read(freshNativeMediaHeadersProvider)(
          url,
        );
        expectStatus(
          'native media read after expiry',
          await _nativeGet(url, headers),
        );
      });
      // The oracle checks the relay saw no session or query traffic, and that
      // a newer assertion was minted, between `expiry_wait` and here.
      fixture.mark('expiry_end');
    }

    session.onAppResumed();
    await step('query_after_expiry', query);

    // Step 4: removing a second community opens a short-lived socket to it
    // to retract its push lease. A relay that enforces NIP-FI refuses that
    // socket without an assertion. The relay's verdict on the retraction
    // itself (its endpoint grant is a stand-in) is outside this check; the
    // oracle only asks that the socket was admitted.
    await step('tombstone', () async {
      final second = fixture.secondOrigin;
      final descriptor = await fetchBuzzPushLeaseDescriptor(second);
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(const MethodChannel('buzz/push'), (
        call,
      ) async {
        expect(call.method, 'endpointGrants');
        return [
          {
            'relayOrigin': descriptor.origin,
            'relayPubkey': 'a' * 64,
            'installationId': 'c' * 32,
            'endpointGrant': 'grant',
            'endpointHash': 'b' * 64,
            'endpointEpoch': 1,
            'generation': 1,
            'expiresAt': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
          },
        ];
      });
      addTearDown(
        () => messenger.setMockMethodCallHandler(
          const MethodChannel('buzz/push'),
          null,
        ),
      );
      await container.read(communityPushLeaseDeactivatorProvider)(
        Community(
          id: 'removed',
          name: 'Removed',
          addedAt: DateTime.utc(2026),
          relayUrl: second.replaceFirst('https', 'wss'),
          pubkey: fixture.pubkey,
          nsec: fixture.nsec,
        ),
        generation: 9,
      );
    });

    fixture.mark('journey_end');
    expect(problems, isEmpty);
  }, timeout: const Timeout(Duration(minutes: 3)));
}

/// A plain `dart:io` GET, as a native media reader would make with the headers
/// it was handed.
Future<int> _nativeGet(String url, Map<String, String> headers) async {
  final client = HttpClient();
  try {
    final request = await client.getUrl(Uri.parse(url));
    headers.forEach(request.headers.set);
    final response = await request.close();
    await response.drain<void>();
    return response.statusCode;
  } finally {
    client.close(force: true);
  }
}

class _FixtureRelayConfig extends RelayConfigNotifier {
  _FixtureRelayConfig(this._origin, this._nsec);
  final String _origin;
  final String _nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: _origin, nsec: _nsec);
}

class _AuthenticatedAuth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

// A 1x1 PNG.
final Uint8List _png = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
);
