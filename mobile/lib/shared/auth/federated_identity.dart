/// NIP-FI integration sketch. Signing stays local; this session supplies only
/// enterprise admission evidence. Adapter exchange and renewal UI are pending.
library;

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

/// Called at actual connection/request time, not when a widget was constructed.
typedef FederatedHeaders =
    Map<String, String> Function(String url, String? nsec);

class FederatedIdentitySession {
  FederatedIdentitySession({required Iterable<String> origins})
    : _origins = origins.map(_origin).toSet();

  final Set<String> _origins;
  int _generation = 0;
  String? _jwt;
  String? _pubkey;
  DateTime? _expires;

  int get generation => _generation;

  /// Login replacement/logout must also cancel sockets and pending operations.
  void invalidate() {
    _generation++;
    _jwt = null;
    _pubkey = null;
    _expires = null;
  }

  /// Input is a trusted adapter response, not renderer/user supplied claims.
  /// This is not a JWT verifier; the relay verifies signature and policy.
  void install({
    required int generation,
    required String jwt,
    required String pubkey,
    required DateTime expires,
    DateTime? now,
  }) {
    if (generation != _generation) throw StateError('Enterprise login changed');
    if (jwt.length > 16384 ||
        !RegExp(
          r'^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$',
        ).hasMatch(jwt) ||
        !RegExp(r'^[0-9a-f]{64}$').hasMatch(pubkey)) {
      throw const FormatException('Invalid enterprise assertion response');
    }
    if (!expires.isAfter(now ?? DateTime.now())) {
      throw StateError('Enterprise assertion expired');
    }
    _jwt = jwt;
    _pubkey = pubkey;
    _expires = expires;
  }

  Map<String, String> headers(String url, String? nsec, {DateTime? now}) {
    if (_origins.isEmpty || !_origins.contains(_origin(url))) return const {};
    final jwt = _jwt;
    if (jwt == null) throw StateError('Enterprise sign-in required');
    if (!_expires!.isAfter(now ?? DateTime.now())) {
      throw StateError('Enterprise assertion expired');
    }
    if (nsec == null ||
        nostr.Keys(nostr.Nip19.decode(payload: nsec).data).public != _pubkey) {
      throw StateError('Enterprise assertion does not match signing identity');
    }
    return {'Nostr-Federated-Identity': 'Bearer $jwt'};
  }

  static String _origin(String value) {
    final uri = Uri.parse(value);
    final scheme = switch (uri.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      final other => other,
    };
    if (uri.userInfo.isNotEmpty ||
        uri.hasFragment ||
        uri.host.isEmpty ||
        (scheme != 'https' &&
            !(scheme == 'http' &&
                ['127.0.0.1', 'localhost', '::1'].contains(uri.host)))) {
      throw const FormatException('Invalid enterprise destination');
    }
    return uri.replace(scheme: scheme).origin;
  }

  @override
  String toString() => 'FederatedIdentitySession([REDACTED])';
}

/// Build-owned destinations. A token must never select its own trusted hosts.
final federatedIdentityProvider = Provider<FederatedIdentitySession>((ref) {
  const configured = String.fromEnvironment('BUZZ_BUILD_NIP_FI_ORIGINS');
  final session = FederatedIdentitySession(
    origins: configured.split(',').where((s) => s.isNotEmpty),
  );
  ref.onDispose(session.invalidate);
  return session;
});
