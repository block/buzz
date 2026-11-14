import 'dart:convert';

/// A compact JWS standing in for an issued NIP-FI assertion named [label]:
/// structurally real, with an `iat` so the client can count its lifetime
/// (its `exp` comes from the response's `expires_at`). Empty stays empty, for
/// tests of an unusable token.
String testAssertion(String label) {
  if (label.isEmpty) return '';
  String segment(Object value) =>
      base64Url.encode(utf8.encode(jsonEncode(value))).replaceAll('=', '');
  return '${segment({'alg': 'ES256'})}.${segment({'iat': 0, 'jti': label})}.c2ln';
}
