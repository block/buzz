import 'dart:async';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';

/// The default transport. `dart:io` would inflate gzip responses on the
/// calling isolate, so leave them compressed for the response decoder. A
/// client supplied through `http.runWithClient` is used as is.
http.Client _gzipPassthroughClient() {
  final client = http.Client();
  if (client is! IOClient) return client;
  client.close();
  return IOClient(HttpClient()..autoUncompress = false);
}

/// Owns the reusable HTTP transport used for relay `/query` requests.
///
/// Requests advertise gzip. A response body may therefore come back
/// compressed; decode it with `relayResponseText` or `decodeRelayQueryResponse`.
class RelayHttpQueryClient {
  RelayHttpQueryClient({
    http.Client? client,
    http.Client Function()? clientFactory,
  }) : _injectedClient = client,
       _clientFactory = clientFactory ?? _gzipPassthroughClient;

  final http.Client? _injectedClient;
  final http.Client Function() _clientFactory;
  _ClientGeneration? _currentGeneration;
  final Set<_ClientGeneration> _generations = {};

  Future<http.Response> post(
    Uri url, {
    required Map<String, String> headers,
    required List<int> body,
    required Duration timeout,
    http.Client Function(http.Client, bool Function())? decorate,
  }) async {
    final generation = _injectedClient == null
        ? (_currentGeneration ??= _createGeneration())
        : null;
    generation?.acquire();
    var active = true;
    try {
      final request = http.Request('POST', url)
        ..followRedirects = false
        ..headers['Accept-Encoding'] = 'gzip'
        ..headers.addAll(headers)
        ..bodyBytes = body;
      final transport = _injectedClient ?? generation!.client;
      final client = decorate?.call(transport, () => active) ?? transport;
      return await client
          .send(request)
          .then(http.Response.fromStream)
          .timeout(timeout);
    } on TimeoutException {
      if (identical(_currentGeneration, generation)) {
        _currentGeneration = null;
      }
      generation?.retire();
      rethrow;
    } finally {
      active = false;
      generation?.release();
    }
  }

  void close() {
    for (final generation in _generations.toList()) {
      generation.close();
    }
    _currentGeneration = null;
    _injectedClient?.close();
  }

  _ClientGeneration _createGeneration() {
    late final _ClientGeneration generation;
    generation = _ClientGeneration(
      _clientFactory(),
      onClosed: () => _generations.remove(generation),
    );
    _generations.add(generation);
    return generation;
  }
}

class _ClientGeneration {
  _ClientGeneration(this.client, {required this.onClosed});

  final http.Client client;
  final void Function() onClosed;
  int _activeRequests = 0;
  bool _retired = false;
  bool _closed = false;

  void acquire() {
    assert(!_closed);
    _activeRequests++;
  }

  void release() {
    assert(_activeRequests > 0);
    _activeRequests--;
    _closeIfIdle();
  }

  void retire() {
    _retired = true;
    _closeIfIdle();
  }

  void close() {
    if (_closed) return;
    _closed = true;
    client.close();
    onClosed();
  }

  void _closeIfIdle() {
    if (_retired && _activeRequests == 0) close();
  }
}
