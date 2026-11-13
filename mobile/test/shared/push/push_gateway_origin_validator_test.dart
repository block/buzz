import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import '../../../scripts/validate_push_gateway_origin.dart';

void main() {
  test(
    'iOS build phase allows omission and validates supplied origins',
    () async {
      Future<ProcessResult> buildPhase(
        Map<String, String> environment,
      ) => Process.run(
        '/bin/sh',
        [
          '-c',
          '. ./scripts/require-push-gateway-origin.sh; printf "continued:%s" "\${DART_DEFINES:-}"',
        ],
        includeParentEnvironment: false,
        environment: {
          'PATH': Platform.environment['PATH']!,
          'SRCROOT': '${Directory.current.path}/ios',
          'CONFIGURATION': 'Release',
          ...environment,
        },
      );
      String define(String origin) =>
          base64.encode(utf8.encode('BUZZ_PUSH_GATEWAY_URL=$origin'));
      final absent = await buildPhase({});
      expect(absent.exitCode, 0, reason: '${absent.stderr}');
      expect(absent.stdout, 'continued:');
      final configured = await buildPhase({
        'BUZZ_PUSH_GATEWAY_URL': 'https://push.example',
      });
      expect(configured.exitCode, 0, reason: '${configured.stderr}');
      expect(configured.stdout, 'continued:${define('https://push.example')}');
      for (final configuration in ['Release', 'Profile']) {
        for (final origin in [
          'https://push.example:443',
          'https://push.example:8444',
        ]) {
          for (final gatewayEnvironment in [
            {'BUZZ_PUSH_GATEWAY_URL': origin},
            {'DART_DEFINES': define(origin)},
          ]) {
            final customPort = await buildPhase({
              'CONFIGURATION': configuration,
              ...gatewayEnvironment,
            });
            expect(customPort.exitCode, 0, reason: '${customPort.stderr}');
            expect(customPort.stdout, 'continued:${define(origin)}');
          }
        }
      }
      for (final origin in [
        '',
        'not-a-url',
        'https://push.example/path',
        'http://localhost:8080',
      ]) {
        final invalid = await buildPhase({'DART_DEFINES': define(origin)});
        expect(invalid.exitCode, isNot(0), reason: origin);
      }
      final debug = await buildPhase({
        'CONFIGURATION': 'Debug',
        'DART_DEFINES': define('http://localhost:8080'),
      });
      expect(debug.exitCode, 0, reason: '${debug.stderr}');
    },
    skip: Platform.isWindows,
  );

  test('accepts origin-only HTTP and HTTPS gateway URLs', () {
    for (final value in [
      'https://push.example',
      'https://push.example/',
      'http://localhost:8080',
    ]) {
      expect(isValidPushGatewayOrigin(value), isTrue, reason: value);
    }
  });

  test('requires HTTPS for release and profile builds', () {
    expect(
      isValidPushGatewayOrigin('https://push.example', requireHttps: true),
      isTrue,
    );
    expect(
      isValidPushGatewayOrigin('http://localhost:8080', requireHttps: true),
      isFalse,
    );
    for (final port in [1, 443, 8444, 65535]) {
      expect(
        isValidPushGatewayOrigin(
          'https://push.example:$port',
          requireHttps: true,
        ),
        isTrue,
        reason: 'valid HTTPS port $port',
      );
    }
  });

  test('rejects malformed or non-origin gateway URLs', () {
    for (final value in [
      '',
      'push.example',
      'ftp://push.example',
      'https://push.example/path',
      'https://push.example?token=x',
      'https://push.example#fragment',
      'https://user@push.example',
      'https://push.example:0',
      'https://push.example:65536',
      'https://push.example:70000',
      'https://push.example:invalid',
      'https://user:password@push.example:8444',
      'https://push.example:8444/path',
      'https://push.example:8444?token=x',
      'https://push.example:8444#fragment',
    ]) {
      expect(isValidPushGatewayOrigin(value), isFalse, reason: value);
      expect(
        isValidPushGatewayOrigin(value, requireHttps: true),
        isFalse,
        reason: value,
      );
    }
  });
}
