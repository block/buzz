import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  for (final scheme in ['ws', 'wss', 'http', 'https']) {
    test(
      'opt-out fetches relay metadata over HTTP for $scheme community',
      () async {
        final container = ProviderContainer();
        addTearDown(container.dispose);
        final community = Community.create(
          name: 'Test',
          relayUrl: '$scheme://relay.example:8443',
          nsec: nostr.Keys.generate().nsec,
        );
        final requests = <http.Request>[];
        final client = MockClient((request) async {
          requests.add(request);
          // Stop at the HTTP boundary, before native grants or publication.
          return http.Response('test unavailable', 503);
        });
        addTearDown(client.close);

        await http.runWithClient(() async {
          await expectLater(
            container.read(communityPushLeaseDeactivatorProvider)(
              community,
              generation: 2,
            ),
            throwsA(
              isA<StateError>().having(
                (error) => error.message,
                'message',
                contains('NIP-11 request failed with HTTP 503'),
              ),
            ),
          );
        }, () => client);

        final httpScheme = scheme == 'ws' || scheme == 'http'
            ? 'http'
            : 'https';
        expect(
          requests.single.url,
          Uri.parse('$httpScheme://relay.example:8443/'),
        );
        expect(requests.single.headers['Accept'], 'application/nostr+json');
      },
    );
  }
}
