import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../relay/relay_provider.dart';
import 'builderlab_account.dart';
import 'account_identity_store.dart';
import 'hosted_environment.dart';

final builderlabAccountProvider = Provider<BuilderlabAccountClient>((ref) {
  final account = BuilderlabAccountClient();
  ref.onDispose(account.dispose);
  return account;
});

final hostedRelayAuthProvider = Provider<HostedRelayAuth>((ref) {
  final auth = HostedRelayAuth(ref.watch(builderlabAccountProvider));
  ref.onDispose(auth.dispose);
  return auth;
});

/// Each transport owns its client lifetime; configuration and auth are shared.
final hostedHttpClientFactoryProvider = Provider<http.Client Function()>((ref) {
  final auth = ref.watch(hostedRelayAuthProvider);
  final config = ref.watch(relayConfigProvider);
  return () => HostedRelayHttpClient(auth, config);
});

/// One account session, with a short-lived assertion for the current relay/key.
/// Enable alongside backend issuance; legacy and self-hosted builds stay usable.
class HostedRelayAuth extends ChangeNotifier {
  HostedRelayAuth(
    this.account, {
    this.enabled = const bool.fromEnvironment('BUZZ_HOSTED_RELAY_AUTH'),
    DateTime Function()? now,
  }) : _now = now ?? DateTime.now;

  final BuilderlabAccountClient account;
  final bool enabled;
  final DateTime Function() _now;
  ({
    String relay,
    String pubkey,
    String token,
    DateTime expiry,
    BuilderlabAccount account,
  })?
  _cached;
  Future<void>? _pending;
  bool _disposed = false;
  String? failedRelay;
  String? failedPubkey;
  AccountFailure? failure;

  bool protects(RelayConfig config) =>
      enabled &&
      HostedEnvironment.requiresAuth(Uri.parse(config.baseUrl).host) &&
      HostedEnvironment.supportsRelay(Uri.parse(config.baseUrl));

  Future<Map<String, String>> headers(RelayConfig config) async {
    if (!protects(config)) return const {};
    final relay = Uri.parse(config.wsUrl).replace(path: '').toString();
    final pubkey = pubkeyFromNsec(config.nsec);
    if (pubkey == null) throw AccountFailure.denied;
    while (!_disposed) {
      final cached = _cached;
      if (cached != null &&
          cached.relay == relay &&
          cached.pubkey == pubkey &&
          identical(cached.account, account.account) &&
          cached.expiry.isAfter(_now())) {
        return {'Nostr-Federated-Identity': 'Bearer ${cached.token}'};
      }
      final pending = _pending;
      if (pending != null) {
        // Different communities share account restoration, never assertions.
        try {
          await pending;
        } on AccountFailure {
          if (failedRelay == config.baseUrl && failedPubkey == pubkey) rethrow;
        }
        continue;
      }
      final operation = _issue(config, relay, pubkey);
      _pending = operation;
      try {
        await operation;
      } finally {
        _pending = null;
      }
    }
    throw AccountFailure.unavailable;
  }

  Future<void> _issue(RelayConfig config, String relay, String pubkey) async {
    try {
      if (account.account == null && await account.restore() == null) {
        throw AccountFailure.signedOut;
      }
      final session = account.account!;
      // Issuance can create a binding. Reconnect must not silently give a
      // different, unbound account this community's saved identity.
      final current = await account.currentIdentity();
      if (!identical(session, account.account)) throw AccountFailure.denied;
      final identity = current['identity'];
      if (current['error'] != null) throw AccountFailure.invalidResponse;
      if (identity == null) {
        final AccountIdentitySnapshot? saved;
        try {
          saved = await AccountIdentityStore()
              .forSubject(session.subject)
              .read();
        } on AccountIdentityFailure {
          throw AccountFailure.storage;
        }
        if (saved?.pubkey != pubkey ||
            !saved!.hasBackupFor(
              backend: BuilderlabAccountClient.origin.origin,
              subject: session.subject,
            )) {
          throw AccountFailure.denied;
        }
      } else if (identity is! Map || identity['pubkey_hex'] != pubkey) {
        throw AccountFailure.denied;
      }
      final value = await account.relayAssertion(
        relay,
        config.nsec!,
        expectedSubject: session.subject,
      );
      final token = value['assertion'];
      final expiry = value['expires_at'];
      if (token is! String ||
          token.isEmpty ||
          token.length > 16384 ||
          token.contains(RegExp(r'[\r\n]')) ||
          value['nostr_pubkey'] != pubkey ||
          expiry is! int) {
        throw AccountFailure.invalidResponse;
      }
      final now = _now().millisecondsSinceEpoch ~/ 1000;
      if (expiry <= now || expiry > now + 300) {
        throw AccountFailure.invalidResponse;
      }
      final expires = DateTime.fromMillisecondsSinceEpoch(
        expiry * 1000,
        isUtc: true,
      );
      if (_disposed) throw AccountFailure.unavailable;
      if (!identical(session, account.account)) {
        return;
      }
      _cached = (
        relay: relay,
        pubkey: pubkey,
        token: token,
        expiry: expires.subtract(
          Duration(seconds: (expiry - now) > 60 ? 30 : (expiry - now) ~/ 2),
        ),
        account: session,
      );
      // Issuance alone cannot prove admission after a relay refusal.
      if (failure != AccountFailure.denied) connected(config);
    } on AccountFailure catch (error) {
      if (!_disposed) {
        failedRelay = config.baseUrl;
        failedPubkey = pubkey;
        failure = error;
        notifyListeners();
      }
      rethrow;
    }
  }

  /// Clear a scoped failure only after this community actually reconnects.
  void connected(RelayConfig config) {
    if (_disposed || (failedRelay != null && failedRelay != config.baseUrl)) {
      return;
    }
    failure = null;
    failedRelay = null;
    failedPubkey = null;
    notifyListeners();
  }

  void rejected(
    RelayConfig config,
    String? usedToken, {
    bool terminal = false,
  }) {
    if (!protects(config) || usedToken == null) return;
    if ('Bearer ${_cached?.token}' == usedToken) _cached = null;
    if (terminal && !_disposed) {
      failedRelay = config.baseUrl;
      failedPubkey = pubkeyFromNsec(config.nsec);
      failure = AccountFailure.denied;
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _cached = null;
    super.dispose();
  }
}

/// Captures the signing context with the request. Never follows redirects with
/// credentials or replays a failed write automatically.
class HostedRelayHttpClient extends http.BaseClient {
  HostedRelayHttpClient(
    this.auth,
    this.config, {
    http.Client? inner,
    this.canSend,
  }) : _inner = inner ?? http.Client();
  final HostedRelayAuth auth;
  final RelayConfig config;
  final http.Client _inner;
  final bool Function()? canSend;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    final origin = Uri.parse(config.baseUrl);
    final scoped = request.url.origin == origin.origin && auth.protects(config);
    if (scoped) {
      request.followRedirects = false;
      request.headers.addAll(await auth.headers(config));
    }
    if (canSend != null && !canSend!()) {
      throw StateError('Relay request expired');
    }
    final response = await _inner.send(request);
    if (scoped && (response.statusCode == 401 || response.statusCode == 403)) {
      auth.rejected(config, request.headers['Nostr-Federated-Identity']);
    }
    return response;
  }

  @override
  void close() => _inner.close();
}
