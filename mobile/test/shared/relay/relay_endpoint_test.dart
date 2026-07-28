import 'package:buzz/shared/relay/relay_endpoint.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('relayEndpoint', () {
    test('joins onto an origin-rooted relay unchanged', () {
      expect(relayEndpoint('https://host', '/query'), 'https://host/query');
      expect(
        relayEndpoint('https://host/', '/media/upload'),
        'https://host/media/upload',
      );
    });

    test('preserves a base path so BUZZ_BASE_PATH deployments resolve', () {
      expect(
        relayEndpoint('https://host/relay', '/query'),
        'https://host/relay/query',
      );
      expect(
        relayEndpoint('https://host/relay/', 'query'),
        'https://host/relay/query',
      );
      expect(
        relayEndpoint('https://host/buzz/relay', '/media/upload'),
        'https://host/buzz/relay/media/upload',
      );
    });

    test('treats the path as relative regardless of leading slashes', () {
      expect(
        relayEndpoint('https://host/relay', 'upload'),
        'https://host/relay/upload',
      );
      expect(
        relayEndpoint('https://host/relay', '//upload'),
        'https://host/relay/upload',
      );
    });

    test('returns the bare base for an empty path', () {
      expect(relayEndpoint('https://host/relay/', '/'), 'https://host/relay');
    });
  });

  group('isRelayMediaPath', () {
    test('accepts the media route at the root and under a prefix', () {
      expect(isRelayMediaPath('/media/abc'), isTrue);
      expect(isRelayMediaPath('/relay/media/abc'), isTrue);
    });

    test('rejects near-miss segments', () {
      expect(isRelayMediaPath('/media-evil/abc'), isFalse);
      expect(isRelayMediaPath('/other/abc'), isFalse);
    });
  });
}
