import 'package:buzz/shared/client/client_headers.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('buildClientHeaders', () {
    test('builds canonical iOS headers and escapes structured strings', () {
      final headers = buildClientHeaders(
        const ClientHeaderMetadata(
          platform: 'ios',
          appVersion: '0.4.5',
          appBuild: '6',
          osVersion: '18.5"preview',
        ),
      );

      expect(
        headers.buzzClient,
        r'v=1, app=buzz-mobile, platform=ios, app-version="0.4.5", '
        r'app-build="6", os-version="18.5\"preview"',
      );
      expect(headers.appVersion, '0.4.5');
      expect(headers.userAgent, 'Buzz/0.4.5 (ios; build 6)');
    });

    test('builds canonical Android headers with the API level', () {
      final headers = buildClientHeaders(
        const ClientHeaderMetadata(
          platform: 'android',
          appVersion: '0.4.5',
          appBuild: '6',
          osVersion: '15',
          osApi: 35,
        ),
      );

      expect(
        headers.buzzClient,
        'v=1, app=buzz-mobile, platform=android, '
        'app-version="0.4.5", app-build="6", os-version="15", os-api=35',
      );
      expect(headers.userAgent, 'Buzz/0.4.5 (android; build 6)');
    });

    test('exposes an inert empty header value', () {
      expect(ClientHeaders.empty.appVersion, isEmpty);
      expect(ClientHeaders.empty.values, isEmpty);
      expect(ClientHeaders.empty.userAgentValue, isEmpty);
    });

    test('partially initialized values fail loudly for every header view', () {
      const partial = ClientHeaders(
        appVersion: '1',
        buzzClient: 'client',
        userAgent: '',
      );
      expect(() => partial.values, throwsStateError);
      expect(() => partial.userAgentValue, throwsStateError);
    });

    test('rejects app versions outside the relay metric contract', () {
      for (final version in ['', '1', '1.2.3.4', '1.2-beta', '123456.2']) {
        expect(
          () => buildClientHeaders(
            ClientHeaderMetadata(
              platform: 'ios',
              appVersion: version,
              appBuild: '1',
              osVersion: '18.0',
            ),
          ),
          throwsFormatException,
          reason: version,
        );
      }
    });

    test('rejects non-decimal app builds', () {
      for (final build in ['', '1.beta', r'6\beta']) {
        expect(
          () => buildClientHeaders(
            ClientHeaderMetadata(
              platform: 'ios',
              appVersion: '1.0',
              appBuild: build,
              osVersion: '18.0',
            ),
          ),
          throwsFormatException,
          reason: build,
        );
      }
    });

    test('rejects invalid platform-specific metadata', () {
      for (final osApi in [null, 0, -1]) {
        expect(
          () => buildClientHeaders(
            ClientHeaderMetadata(
              platform: 'android',
              appVersion: '1.0',
              appBuild: '1',
              osVersion: '15',
              osApi: osApi,
            ),
          ),
          throwsArgumentError,
          reason: '$osApi',
        );
      }
      expect(
        () => buildClientHeaders(
          const ClientHeaderMetadata(
            platform: 'ios',
            appVersion: '1.0',
            appBuild: '1',
            osVersion: '18.0',
            osApi: 35,
          ),
        ),
        throwsArgumentError,
      );
    });
  });

  group('first-party origin gating', () {
    const headers = ClientHeaders(
      appVersion: '1.0',
      buzzClient: 'client',
      userAgent: 'agent',
    );

    test('allows the configured relay across HTTP and WebSocket schemes', () {
      expect(
        clientHeadersForUrl(
          headers: headers,
          targetUrl: 'wss://relay.example:443/socket',
          relayUrl: 'https://Relay.Example/base',
          allowLocalDevelopment: false,
        ),
        {'Buzz-Client': 'client', 'User-Agent': 'agent'},
      );
    });

    test('allows hosted communities and the dedicated pairing relay', () {
      for (final url in [
        'wss://acme.communities.buzz.xyz',
        'https://PAIRING.BUZZ.XYZ/path',
      ]) {
        expect(
          isFirstPartyBuzzUrl(url, allowLocalDevelopment: false),
          isTrue,
          reason: url,
        );
      }
    });

    test('sends only User-Agent to other HTTP and WebSocket servers', () {
      for (final url in [
        'https://cdn.cloudflare.example/attachment.png',
        'wss://relay.other.example/socket',
        'https://communities.buzz.xyz.evil.example/media/file',
        'ws://pairing.buzz.xyz',
        'wss://relay.example:444/socket',
      ]) {
        expect(
          clientHeadersForUrl(
            headers: headers,
            targetUrl: url,
            relayUrl: 'https://relay.example',
            allowLocalDevelopment: false,
          ),
          {'User-Agent': 'agent'},
          reason: url,
        );
      }
    });

    test('does not send headers to malformed or non-network URLs', () {
      for (final url in ['', 'not a URL', 'file:///tmp/media.png']) {
        expect(
          clientHeadersForUrl(
            headers: headers,
            targetUrl: url,
            relayUrl: 'https://relay.example',
            allowLocalDevelopment: false,
          ),
          isEmpty,
          reason: url,
        );
      }
    });
  });
}
