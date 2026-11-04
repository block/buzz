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
}
