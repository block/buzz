import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;

void main() {
  test('fetches the NIP-11 icon over the relay HTTP URL', () async {
    late http.Request capturedRequest;
    final client = http_testing.MockClient((request) async {
      capturedRequest = request;
      return http.Response('{"icon":"data:image/png;base64,icon-bytes"}', 200);
    });
    final container = ProviderContainer(
      overrides: [communityIconHttpClientProvider.overrideWithValue(client)],
    );
    addTearDown(container.dispose);

    final icon = await container.read(
      communityIconProvider('wss://relay.example.com').future,
    );

    expect(capturedRequest.url, Uri.parse('https://relay.example.com'));
    expect(capturedRequest.headers['Accept'], 'application/nostr+json');
    expect(icon, 'data:image/png;base64,icon-bytes');
  });

  test('returns null when the relay has no usable icon', () async {
    final client = http_testing.MockClient(
      (_) async => http.Response('{"icon":""}', 200),
    );
    final container = ProviderContainer(
      overrides: [communityIconHttpClientProvider.overrideWithValue(client)],
    );
    addTearDown(container.dispose);

    final icon = await container.read(
      communityIconProvider('https://relay.example.com').future,
    );

    expect(icon, isNull);
  });

  test('returns null when relay information cannot be loaded', () async {
    final client = http_testing.MockClient(
      (_) async => http.Response('unavailable', 503),
    );
    final container = ProviderContainer(
      overrides: [communityIconHttpClientProvider.overrideWithValue(client)],
    );
    addTearDown(container.dispose);

    final icon = await container.read(
      communityIconProvider('https://relay.example.com').future,
    );

    expect(icon, isNull);
  });
}
