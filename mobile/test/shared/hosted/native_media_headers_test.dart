import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/native_media_headers.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

final _nsec = nostr.Keys.generate().nsec;

class _RelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

/// A hosted relay whose live session already holds an assertion.
class _CachedHostedRelayAuth extends HostedRelayAuth {
  _CachedHostedRelayAuth() : super(BuilderlabAccountClient(), enabled: false);

  @override
  Map<String, String>? cachedHeaders(RelayConfig config) => {
    'Nostr-Federated-Identity': 'Bearer hosted-token',
  };
}

/// A hosted relay whose cached assertion has expired: only a fresh request
/// gets one, and [fail] makes that request fail.
class _ExpiredHostedRelayAuth extends HostedRelayAuth {
  _ExpiredHostedRelayAuth({this.fail = false})
    : super(BuilderlabAccountClient(), enabled: false);

  final bool fail;
  final requested = <Uri>[];

  @override
  Map<String, String>? cachedHeaders(RelayConfig config) => null;

  @override
  Future<Map<String, String>> headersForUrl(RelayConfig config, Uri url) async {
    requested.add(url);
    if (fail) throw AccountFailure.unavailable;
    return {'Nostr-Federated-Identity': 'Bearer fresh-token'};
  }
}

ProviderContainer _container(HostedRelayAuth hosted) => ProviderContainer(
  overrides: [
    relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
    mediaGetAuthServiceProvider.overrideWithValue(
      MediaGetAuthService(baseUrl: 'https://relay.example', nsec: _nsec),
    ),
    hostedRelayAuthProvider.overrideWithValue(hosted),
  ],
);

void main() {
  test('native media headers add the relay assertion only for relay media', () {
    final hosted = _CachedHostedRelayAuth();
    addTearDown(hosted.dispose);
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
        mediaGetAuthServiceProvider.overrideWithValue(
          MediaGetAuthService(baseUrl: 'https://relay.example', nsec: _nsec),
        ),
        hostedRelayAuthProvider.overrideWithValue(hosted),
      ],
    );
    addTearDown(container.dispose);
    final headersFor = container.read(nativeMediaHeadersProvider);

    final relay = headersFor('https://relay.example/media/a.png');
    expect(relay['Authorization'], startsWith('Nostr '));
    expect(relay['Nostr-Federated-Identity'], 'Bearer hosted-token');

    for (final other in [
      'https://cdn.example/a.png',
      'https://team.relay.example/media/a.png',
      'http://relay.example/media/a.png',
      'https:///media/a.png',
      'http:///media/a.png',
      'https://',
    ]) {
      expect(headersFor(other), isNot(contains('Nostr-Federated-Identity')));
    }
  });

  test('fresh native media headers renew an expired assertion', () async {
    const url = 'https://relay.example/media/voice.m4a';
    final hosted = _ExpiredHostedRelayAuth();
    addTearDown(hosted.dispose);
    final container = _container(hosted);
    addTearDown(container.dispose);

    // The synchronous form can only offer what is cached.
    expect(
      container.read(nativeMediaHeadersProvider)(url),
      isNot(contains('Nostr-Federated-Identity')),
    );

    final headers = await container.read(freshNativeMediaHeadersProvider)(url);
    expect(headers['Authorization'], startsWith('Nostr '));
    expect(headers['Nostr-Federated-Identity'], 'Bearer fresh-token');
    expect(hosted.requested, [Uri.parse(url)]);

    // A failed renewal still sends the media proof, and the relay decides.
    final failing = _ExpiredHostedRelayAuth(fail: true);
    addTearDown(failing.dispose);
    final failingContainer = _container(failing);
    addTearDown(failingContainer.dispose);
    final fallback = await failingContainer.read(
      freshNativeMediaHeadersProvider,
    )(url);
    expect(fallback['Authorization'], startsWith('Nostr '));
    expect(fallback, isNot(contains('Nostr-Federated-Identity')));
  });
}
