import 'package:flutter_test/flutter_test.dart';

import '../../../scripts/validate_push_gateway_origin.dart';

void main() {
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
    expect(
      isValidPushGatewayOrigin('https://push.example:8443', requireHttps: true),
      isFalse,
    );
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
      'https://push.example:70000',
    ]) {
      expect(isValidPushGatewayOrigin(value), isFalse, reason: value);
    }
  });
}
