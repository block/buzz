import 'dart:convert';

import 'package:http/http.dart' as http;

import '../http/app_http_client.dart';

/// The gateway answered with a refusal. Signed installation calls answer every
/// refusal with `404 not_authorized`, so [status] and [code] say what the
/// gateway said and nothing more.
class AndroidPushGatewayRejection implements Exception {
  const AndroidPushGatewayRejection(this.route, this.status, this.code);

  final String route;
  final int status;
  final String? code;

  bool get isNotAuthorized => status == 404 && code == 'not_authorized';

  /// A refusal of the attestation itself, which only a different phone or boot
  /// state changes.
  bool get isInvalidAttestation =>
      status == 401 && code == 'invalid_attestation';

  /// Refusals worth retrying later with the same request: throttling and
  /// gateway-side trouble. Every other refusal is final for that request.
  bool get isTransient => status == 429 || status >= 500;

  @override
  String toString() => 'Push gateway refused $route: $status ${code ?? ''}';
}

/// One round trip to the gateway's JSON API.
class AndroidPushGatewayClient {
  AndroidPushGatewayClient({
    required this.baseUrl,
    http.Client? client,
    this.timeout = const Duration(seconds: 15),
  }) : _client = client ?? newAppHttpClient();

  final Uri baseUrl;
  final Duration timeout;
  final http.Client _client;

  /// Posts [body] to [route] and returns the JSON object answered with
  /// [expectedStatus]. Transport failures propagate unchanged.
  Future<Map<String, Object?>> post(
    String route,
    Map<String, Object?> body, {
    int expectedStatus = 200,
  }) async {
    final response = await _client
        .post(
          baseUrl.resolve(route),
          headers: const {'content-type': 'application/json'},
          body: jsonEncode(body),
        )
        .timeout(timeout);
    final decoded = _object(response.bodyBytes);
    if (response.statusCode != expectedStatus) {
      throw AndroidPushGatewayRejection(
        route,
        response.statusCode,
        decoded?['error'] as String?,
      );
    }
    if (decoded == null) {
      throw FormatException(
        'Push gateway answered $route without a JSON object.',
      );
    }
    return decoded;
  }

  Map<String, Object?>? _object(List<int> bytes) {
    try {
      final value = jsonDecode(utf8.decode(bytes));
      return value is Map<String, Object?> ? value : null;
    } on FormatException {
      return null;
    }
  }

  void close() => _client.close();
}
