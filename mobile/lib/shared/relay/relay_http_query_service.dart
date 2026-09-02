import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:http/http.dart' as http;

import 'nostr_models.dart';
import 'relay_client.dart';
import 'relay_closed_policy.dart';
import 'relay_http_query_client.dart';
import 'relay_provider.dart';
import 'relay_rate_limit_gate.dart';

typedef Nip98AuthHeaderBuilder =
    String Function({
      required String method,
      required String url,
      required List<int> bodyBytes,
      required String? nsec,
    });

/// Executes relay HTTP queries with an optional short-lived LAN fast path.
class RelayHttpQueryService {
  RelayHttpQueryService({
    http.Client? client,
    http.Client Function()? clientFactory,
    required DateTime Function() now,
    required Nip98AuthHeaderBuilder authHeaderBuilder,
  }) : _client = RelayHttpQueryClient(
         client: client,
         clientFactory: clientFactory,
       ),
       _now = now,
       _authHeaderBuilder = authHeaderBuilder;

  static const _lanProbeTimeout = Duration(seconds: 2);
  static const _lanRetryDelay = Duration(seconds: 30);

  final RelayHttpQueryClient _client;
  final DateTime Function() _now;
  final Nip98AuthHeaderBuilder _authHeaderBuilder;

  String? _unavailableLanBaseUrl;
  DateTime? _lanRetryAt;

  Future<List<NostrEvent>> query(
    RelayConfig config,
    List<NostrFilter> filters, {
    required RelayRateLimitGate rateLimitGate,
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final bodyBytes = utf8.encode(
      jsonEncode(filters.map((filter) => filter.toJson()).toList()),
    );
    final baseUrls = _availableBaseUrls(config);
    Object? lastTransportError;

    for (var index = 0; index < baseUrls.length; index++) {
      final baseUrl = baseUrls[index];
      final hasFallback = index + 1 < baseUrls.length;
      final isLanCandidate = baseUrl != config.baseUrl;
      final requestUrl = Uri.parse(baseUrl).resolve('/query').toString();
      final canonicalUrl = Uri.parse(
        config.baseUrl,
      ).resolve('/query').toString();
      final requestTimeout = isLanCandidate && timeout > _lanProbeTimeout
          ? _lanProbeTimeout
          : timeout;

      try {
        final response = await _client.post(
          Uri.parse(requestUrl),
          headers: {
            if (isLanCandidate)
              HttpHeaders.hostHeader: Uri.parse(config.baseUrl).authority,
            'Authorization': _authHeaderBuilder(
              method: 'POST',
              url: canonicalUrl,
              bodyBytes: bodyBytes,
              nsec: config.nsec,
            ),
            'Content-Type': 'application/json',
          },
          body: bodyBytes,
          timeout: requestTimeout,
        );
        if (response.statusCode < 200 || response.statusCode >= 300) {
          _activateRateLimitGate(response.body, rateLimitGate);
          final error = RelayException(response.statusCode, response.body);
          if (isLanCandidate &&
              hasFallback &&
              (response.statusCode == 404 || response.statusCode >= 500)) {
            _markLanUnavailable(baseUrl);
            lastTransportError = error;
            continue;
          }
          throw error;
        }
        if (isLanCandidate) _clearLanFailure();
        return _decodeEvents(response.body);
      } on TimeoutException catch (error) {
        if (!isLanCandidate || !hasFallback) rethrow;
        _markLanUnavailable(baseUrl);
        lastTransportError = error;
      } on http.ClientException catch (error) {
        if (!isLanCandidate || !hasFallback) rethrow;
        _markLanUnavailable(baseUrl);
        lastTransportError = error;
      }
    }

    throw lastTransportError ?? StateError('No relay HTTP transport available');
  }

  void close() => _client.close();

  List<String> _availableBaseUrls(RelayConfig config) {
    final candidates = config.httpBaseUrls;
    if (candidates.length < 2) return candidates;
    final lanBaseUrl = candidates.first;
    if (_unavailableLanBaseUrl != lanBaseUrl) return candidates;
    final retryAt = _lanRetryAt;
    if (retryAt == null || !_now().isBefore(retryAt)) return candidates;
    return [config.baseUrl];
  }

  void _markLanUnavailable(String baseUrl) {
    _unavailableLanBaseUrl = baseUrl;
    _lanRetryAt = _now().add(_lanRetryDelay);
  }

  void _clearLanFailure() {
    _unavailableLanBaseUrl = null;
    _lanRetryAt = null;
  }

  List<NostrEvent> _decodeEvents(String body) {
    final decoded = jsonDecode(body);
    if (decoded is! List) {
      throw const FormatException('relay returned malformed query response');
    }
    try {
      return [
        for (final eventJson in decoded)
          if (eventJson is Map<String, dynamic>)
            NostrEvent.fromJson(eventJson)
          else
            throw const FormatException('relay returned malformed query event'),
      ];
    } catch (error) {
      if (error is FormatException) rethrow;
      throw FormatException('relay returned malformed query event: $error');
    }
  }

  void _activateRateLimitGate(String body, RelayRateLimitGate rateLimitGate) {
    final dynamic decoded;
    try {
      decoded = jsonDecode(body);
    } on FormatException {
      return;
    }
    if (decoded is! Map<String, dynamic>) return;
    final message = decoded['error'];
    if (message is! String ||
        classifyRelayClosed(message) != RelayClosedClass.rateLimited) {
      return;
    }
    rateLimitGate.activate(parseRateLimitRetrySeconds(message));
  }
}
