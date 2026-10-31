import 'dart:async';
import 'dart:io';

/// Bounds how long a relay WebSocket may take to connect and upgrade.
///
/// The OS gives up on a stalled TCP or TLS handshake only after minutes, and
/// reconnecting starts only once this attempt fails, so an unbounded attempt
/// leaves the app reconnecting for as long as the network stays stalled.
const relayWebSocketConnectTimeout = Duration(seconds: 10);

/// WebSocket's default HTTP client follows redirects, including custom headers.
/// Keep key-bound assertions at their original origin and bound upgrade time.
Future<WebSocket> openRelayWebSocket(
  Uri uri, {
  required Map<String, String> headers,
  required Duration pingInterval,
  Duration connectTimeout = relayWebSocketConnectTimeout,
}) async {
  if (headers.isEmpty) {
    final connecting = WebSocket.connect(uri.toString());
    return connecting
        .then((socket) => socket..pingInterval = pingInterval)
        .timeout(
          connectTimeout,
          onTimeout: () {
            // The default client is shared, so close only a socket that
            // finishes connecting after the caller has given up on it.
            unawaited(
              connecting.then((late) => late.close(), onError: (_) => null),
            );
            throw TimeoutException('Relay connection timed out');
          },
        );
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
          connectTimeout,
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
