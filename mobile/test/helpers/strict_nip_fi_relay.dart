import 'dart:convert';
import 'dart:io';

/// The NIP-FI check an enforcing relay makes before it answers, for test
/// doubles. A request passes only with a `Nostr-Federated-Identity` bearer
/// assertion that the test issuer minted for this relay and that is unexpired
/// at the relay's clock. Doubles refuse anything else with a 401 and record
/// why, so a client that leaves the header off, sends another relay's
/// assertion or reuses an expired one fails the test that drove it.
class StrictNipFiRelay {
  StrictNipFiRelay({DateTime Function()? now}) : _now = now ?? DateTime.now;

  static const header = 'Nostr-Federated-Identity';

  final DateTime Function() _now;
  final _expiries = <(String, String), DateTime>{};

  /// Refused requests, as `route: reason`.
  final refusals = <String>[];

  /// The headers of every socket upgrade [serve] saw, admitted or not.
  final upgrades = <HttpHeaders>[];

  /// Records [assertion] as minted for [relayUrl] until [expiresAt] (Unix
  /// seconds), and returns it for the issuer's response.
  String issue(
    String assertion, {
    required String relayUrl,
    required int expiresAt,
  }) {
    _expiries[(Uri.parse(relayUrl).authority, assertion)] =
        DateTime.fromMillisecondsSinceEpoch(expiresAt * 1000);
    return assertion;
  }

  /// Whether [url]'s relay admits a request carrying [value] as its
  /// [header]. A refusal is recorded under [route], or the URL's path.
  bool admits(Uri url, String? value, {String? route}) {
    const scheme = 'Bearer ';
    final token = value != null && value.startsWith(scheme)
        ? value.substring(scheme.length)
        : null;
    final expiry = token == null ? null : _expiries[(url.authority, token)];
    final reason = token == null
        ? 'no assertion'
        : expiry == null
        ? 'an assertion not minted for this relay'
        : _now().isBefore(expiry)
        ? null
        : 'an expired assertion';
    if (reason == null) return true;
    refusals.add('${route ?? url.path}: $reason');
    return false;
  }

  /// Serves a loopback relay that answers NIP-11 with [descriptor] and
  /// upgrades a socket only once it [admits] it, then accepts every AUTH and
  /// EVENT. Tests that use it must clear `HttpOverrides.global` first.
  Future<HttpServer> serve(Object? Function(String origin) descriptor) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final origin = 'ws://127.0.0.1:${server.port}';
    server.listen((request) async {
      if (!WebSocketTransformer.isUpgradeRequest(request)) {
        request.response
          ..headers.contentType = ContentType('application', 'nostr+json')
          ..write(jsonEncode(descriptor(origin)))
          ..close();
        return;
      }
      upgrades.add(request.headers);
      final assertion = request.headers.value(header);
      if (!admits(Uri.parse(origin), assertion, route: 'connect')) {
        request.response
          ..statusCode = HttpStatus.unauthorized
          ..close();
        return;
      }
      final socket = await WebSocketTransformer.upgrade(request);
      socket.add(jsonEncode(['AUTH', 'challenge']));
      socket.listen((message) {
        final frame = jsonDecode(message as String) as List<dynamic>;
        if (frame.first == 'AUTH' || frame.first == 'EVENT') {
          socket.add(jsonEncode(['OK', (frame[1] as Map)['id'], true, '']));
        }
      });
    });
    return server;
  }
}
