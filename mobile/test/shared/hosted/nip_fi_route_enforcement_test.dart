import 'dart:typed_data';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/native_media_headers.dart';
import 'package:buzz/shared/http/app_http_client.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:buzz/shared/relay/media_upload.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import '../../helpers/hosted_journey_fixture.dart';
import '../../helpers/strict_nip_fi_relay.dart';

/// Every relay route class the app sends NIP-FI evidence on, driven through
/// production code against a relay that enforces it. Each route runs after the
/// first assertion expired at the relay, so a route that skips renewal, or
/// drops the header, is refused. The organization realm runs in every build.
void main() {
  const host = HostedJourneyFixture.organizationRelayHost;
  final media = 'https://$host/media/${'a' * 64}';

  final routes = <String, Future<void> Function(HostedJourneyFixture)>{
    'connect': (f) async {
      final connects = f.socketHeaders.length;
      await f.container.read(relaySessionProvider.notifier).reconnect();
      expect(f.socketHeaders, hasLength(greaterThan(connects)));
    },
    'query': (f) async {
      await f.container.read(relaySessionProvider.notifier).queryRelay([]);
      expect(f.sent('/query'), isNotEmpty);
    },
    'upload': (f) async {
      final config = f.container.read(relayConfigProvider);
      final client = f.container.read(hostedHttpClientFactoryProvider)(
        traffic: HttpTraffic.media,
      );
      addTearDown(client.close);
      final uploads = MediaUploadService(
        httpClient: client,
        baseUrl: config.baseUrl,
        nsec: config.nsec,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );
      addTearDown(uploads.dispose);
      await uploads.uploadBytes(Uint8List(4), mimeType: 'video/mp4');
    },
    'media': (f) async {
      final response = await f.container
          .read(mediaHttpClientProvider)
          .get(Uri.parse(media));
      expect(response.statusCode, 200);
    },
    // A native player gets its headers up front and sends them itself.
    'native media': (f) async {
      final headers = await f.container.read(freshNativeMediaHeadersProvider)(
        media,
      );
      final response = await http.get(Uri.parse(media), headers: headers);
      expect(response.statusCode, 200);
    },
  };

  Future<void> connected(WidgetTester tester, HostedJourneyFixture f) async {
    f.relayAuthEnabled = true;
    f.organizationEnabled = true;
    await f.initialize();
    f.organizationBound = f.keys.public;
    await f.signInOrganization();
    final community = Community.create(
      name: 'Example',
      relayUrl: 'wss://$host',
      pubkey: f.keys.public,
      nsec: f.keys.nsec,
    );
    await f.storage.save(community);
    await f.storage.saveActiveId(community.id);
    await f.mount(tester, openEntry: false);
    final session = f.container.listen(relaySessionProvider, (_, _) {});
    addTearDown(session.close);
    await f.settle(tester);
    expect(
      f.container.read(relaySessionProvider).status,
      SessionStatus.connected,
    );
  }

  void journey(String name, Future<void> Function(HostedJourneyFixture) run) {
    testWidgets(name, (tester) async {
      final f = HostedJourneyFixture();
      await tester.runAsync(
        () => http.runWithClient(() async {
          try {
            await connected(tester, f);
            await run(f);
            await f.settle(tester);
          } finally {
            await f.dispose(tester);
          }
        }, f.client),
      );
    });
  }

  journey('the relay refuses a missing, foreign or expired assertion', (
    f,
  ) async {
    final cached = f.container
        .read(hostedRelayAuthProvider)
        .cachedHeaders(f.container.read(relayConfigProvider))!;
    final query = Uri.parse('https://$host/query');
    for (final headers in [
      <String, String>{},
      {StrictNipFiRelay.header: 'Bearer foreign'},
    ]) {
      expect((await http.post(query, headers: headers)).statusCode, 401);
    }
    f.assertionNow = f.assertionNow.add(const Duration(minutes: 6));
    expect((await http.post(query, headers: cached)).statusCode, 401);
    expect(f.relay.refusals, [
      '/query: no assertion',
      '/query: an assertion not minted for this relay',
      '/query: an expired assertion',
    ]);
    f.relay.refusals.clear();
  });

  for (final MapEntry(key: name, value: send) in routes.entries) {
    journey('$name renews an expired assertion for an enforcing relay', (
      f,
    ) async {
      f.assertionNow = f.assertionNow.add(const Duration(minutes: 6));
      await send(f);
      expect(f.relay.refusals, isEmpty);
    });
  }
}
