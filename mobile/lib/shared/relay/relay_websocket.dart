import 'dart:async';
import 'dart:io';

/// WebSocket's default HTTP client follows redirects, including custom headers.
/// Keep key-bound assertions at their original origin and bound upgrade time.
Future<WebSocket> openRelayWebSocket(
  Uri uri, {
  required Map<String, String> headers,
  required Duration pingInterval,
}) async {
  if (headers.isEmpty) {
    return WebSocket.connect(
      uri.toString(),
    ).then((socket) => socket..pingInterval = pingInterval);
  }
  final client = _UpgradeClient();
  try {
    return await WebSocket.connect(
          uri.toString(),
          headers: headers,
          customClient: client,
        )
        .then((socket) => socket..pingInterval = pingInterval)
        .timeout(
          const Duration(seconds: 10),
          onTimeout: () {
            client.close(force: true);
            throw TimeoutException('Relay connection timed out');
          },
        );
  } finally {
    client.close();
  }
}

/// WebSocket.connect uses only openUrl on its supplied HTTP client. Keep this
/// adapter limited to upgrades instead of exposing a second general transport.
class _UpgradeClient implements HttpClient {
  final _inner = HttpClient();

  @override
  Future<HttpClientRequest> openUrl(String method, Uri url) async {
    final request = await _inner.openUrl(method, url);
    request.followRedirects = false;
    return request;
  }

  @override
  void close({bool force = false}) => _inner.close(force: force);

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
