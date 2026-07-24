import 'package:http/http.dart' as http;

/// Lightweight HTTP context for talking to the Buzz relay.
///
/// In the pure-nostr architecture, all data flow happens over the relay
/// WebSocket. This client now exists only to provide a base URL (and a
/// shared HTTP client) for the media upload endpoint, which is the one
/// remaining HTTP path because Blossom uses kind:24242 NIP-98 auth on a
/// regular HTTP PUT.
class RelayClient {
  final String baseUrl;
  final http.Client _http;

  RelayClient({required this.baseUrl, http.Client? httpClient})
    : _http = httpClient ?? http.Client();

  /// Shared underlying HTTP client (used by [MediaUploader]).
  http.Client get httpClient => _http;

  /// Fully-qualified URL for the relay's Blossom-style media upload endpoint.
  String get mediaUploadUrl {
    final base = Uri.parse(baseUrl);
    return base.resolve('/upload').toString();
  }

  void dispose() => _http.close();
}

/// Thrown when an HTTP call to the relay returns a non-2xx status code.
///
/// Retained for backwards compatibility with provider code that still
/// references it during the migration to pure-nostr WebSocket flows.
class RelayException implements Exception {
  final int statusCode;
  final String body;

  RelayException(this.statusCode, this.body);

  @override
  String toString() {
    final trimmedBody = body.trim();
    if (trimmedBody.isEmpty) {
      return 'RelayException($statusCode)';
    }
    return 'RelayException($statusCode): $trimmedBody';
  }
}
