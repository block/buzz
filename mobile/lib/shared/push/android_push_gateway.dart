import 'dart:convert';

import 'package:http/http.dart' as http;

import '../hosted/bounded_http_response.dart';
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

/// The gateway answered with something the app cannot read: no JSON object,
/// or one missing a field the call needs. Unlike a refusal, this points at a
/// gateway or protocol defect, so it is reported to diagnostics.
class AndroidPushGatewayResponseInvalid extends FormatException {
  const AndroidPushGatewayResponseInvalid(super.message);
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
  /// [expectedStatus]. Transport failures propagate unchanged. At [timeout]
  /// the request is aborted, not just abandoned, so a retry never races a
  /// copy of itself still on its way to the gateway.
  Future<Map<String, Object?>> post(
    String route,
    Map<String, Object?> body, {
    int expectedStatus = 200,
  }) async {
    final http.Response response;
    int? status;
    try {
      response = await boundedHttpResponse(
        _client,
        http.Request('POST', baseUrl.resolve(route))
          ..headers['content-type'] = 'application/json'
          ..body = jsonEncode(body),
        timeout: timeout,
        onHeaders: (headers) => status = headers.statusCode,
      );
    } on FormatException catch (error) {
      // The only one a bounded read throws: a body over its limit. A refusal
      // keeps its status, so an oversized 503 still reads as transient.
      if (status case final status? when status != expectedStatus) {
        throw AndroidPushGatewayRejection(route, status, null);
      }
      throw AndroidPushGatewayResponseInvalid(
        'Push gateway answered $route unreadably: ${error.message}',
      );
    }
    final decoded = _object(response.bodyBytes);
    if (response.statusCode != expectedStatus) {
      throw AndroidPushGatewayRejection(
        route,
        response.statusCode,
        decoded?['error'] as String?,
      );
    }
    if (decoded == null) {
      throw AndroidPushGatewayResponseInvalid(
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
