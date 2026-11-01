import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:buzz/shared/http/app_http_client.dart';

import '../relay/relay_provider.dart';
import 'builderlab_account.dart';
import 'account_identity_store.dart';
import 'hosted_environment.dart';
import 'organization_environment.dart';

final builderlabAccountProvider = Provider<BuilderlabAccountClient>((ref) {
  final account = BuilderlabAccountClient();
  ref.onDispose(account.dispose);
  return account;
});

/// The organization's own account session, separate from the hosted account.
/// Null unless this build configures [OrganizationEnvironment].
final organizationAccountProvider = Provider<BuilderlabAccountClient?>((ref) {
  final origin = OrganizationEnvironment.origin;
  if (origin == null) return null;
  final account = BuilderlabAccountClient(accountOrigin: origin);
  ref.onDispose(account.dispose);
  return account;
});

final hostedRelayAuthProvider = Provider<HostedRelayAuth>((ref) {
  final auth = HostedRelayAuth(
    ref.watch(builderlabAccountProvider),
    organization: ref.watch(organizationAccountProvider),
  );
  ref.onDispose(auth.dispose);
  return auth;
});

/// Each transport owns its client lifetime; configuration and auth are shared.
final hostedHttpClientFactoryProvider =
    Provider<http.Client Function({HttpTraffic traffic})>((ref) {
      final auth = ref.watch(hostedRelayAuthProvider);
      final config = ref.watch(relayConfigProvider);
      return ({HttpTraffic traffic = HttpTraffic.api}) =>
          HostedRelayHttpClient(auth, config, inner: newAppHttpClient(traffic));
    });

/// One account session per realm, with a short-lived assertion for the current
/// relay/key. Hosted communities use [account] when [enabled]; the build's
/// organization relays use [organization]. Enable alongside backend issuance;
/// legacy and self-hosted builds stay usable.
class HostedRelayAuth extends ChangeNotifier {
  HostedRelayAuth(
    this.account, {
    this.organization,
    this.organizationRelay = OrganizationEnvironment.protects,
    this.enabled = const bool.fromEnvironment('BUZZ_HOSTED_RELAY_AUTH'),
    DateTime Function()? now,
  }) : _now = now ?? DateTime.now;

  final BuilderlabAccountClient account;
  final BuilderlabAccountClient? organization;
  final bool Function(Uri relay) organizationRelay;
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
  // Per account client, so a stalled issuer never delays an unrelated one.
  final _pending = <BuilderlabAccountClient, Future<void>>{};
  bool _disposed = false;
  // Orders issuance and refusals, so a stale result never replaces a newer
  // failure for another relay or key.
  int _sequence = 0;
  int _failureSequence = 0;
  String? failedRelay;
  String? failedPubkey;
  AccountFailure? failure;

  bool protects(RelayConfig config) => _accountFor(config) != null;

  /// Whether this relay uses the organization's session, not the hosted one.
  bool organizationProtects(RelayConfig config) =>
      organization != null && identical(_accountFor(config), organization);

  BuilderlabAccountClient? _accountFor(RelayConfig config) {
    final relay = Uri.parse(config.baseUrl);
    if (organization != null && organizationRelay(relay)) {
      return organization;
    }
    if (enabled &&
        HostedEnvironment.requiresAuth(relay.host) &&
        HostedEnvironment.supportsRelay(relay)) {
      return account;
    }
    return null;
  }

  /// Headers for optional reads, without starting account work or changing
  /// recovery state. Null means protected metadata must wait for active auth.
  Map<String, String>? cachedHeaders(RelayConfig config) {
    if (_disposed) return null;
    if (!protects(config)) return const {};
    final cached = _cached;
    if (cached == null ||
        cached.relay != Uri.parse(config.wsUrl).replace(path: '').toString() ||
        cached.pubkey != pubkeyFromNsec(config.nsec) ||
        !identical(cached.account, _accountFor(config)?.account) ||
        !cached.expiry.isAfter(_now())) {
      return null;
    }
    return {'Nostr-Federated-Identity': 'Bearer ${cached.token}'};
  }

  Future<Map<String, String>> headers(RelayConfig config) async {
    final client = _accountFor(config);
    if (client == null) return const {};
    try {
      return await _headers(config, client);
    } on AccountFailure catch (failure) {
      // Without an organization session, connect as before and let the relay
      // decide: an enforcing relay refuses, and sign-in is offered.
      if (failure == AccountFailure.signedOut &&
          identical(client, organization)) {
        return const {};
      }
      rethrow;
    }
  }

  Future<Map<String, String>> _headers(
    RelayConfig config,
    BuilderlabAccountClient client,
  ) async {
    final relay = Uri.parse(config.wsUrl).replace(path: '').toString();
    final pubkey = pubkeyFromNsec(config.nsec);
    if (pubkey == null) throw AccountFailure.denied;
    while (!_disposed) {
      final cached = _cached;
      if (cached != null &&
          cached.relay == relay &&
          cached.pubkey == pubkey &&
          identical(cached.account, client.account) &&
          cached.expiry.isAfter(_now())) {
        return {'Nostr-Federated-Identity': 'Bearer ${cached.token}'};
      }
      final pending = _pending[client];
      if (pending != null) {
        // Different communities share account restoration, never assertions.
        try {
          await pending;
        } on AccountFailure {
          if (failedRelay == config.baseUrl && failedPubkey == pubkey) rethrow;
        }
        continue;
      }
      final operation = _issue(config, client, relay, pubkey);
      _pending[client] = operation;
      try {
        await operation;
      } finally {
        if (identical(_pending[client], operation)) _pending.remove(client);
      }
    }
    throw AccountFailure.unavailable;
  }

  Future<void> _issue(
    RelayConfig config,
    BuilderlabAccountClient account,
    String relay,
    String pubkey,
  ) async {
    final started = ++_sequence;
    try {
      if (account.account == null && await account.restore() == null) {
        throw AccountFailure.signedOut;
      }
      final session = account.account!;
      final current = await account.currentIdentity();
      if (!identical(session, account.account)) throw AccountFailure.denied;
      final identity = current['identity'];
      if (current['error'] != null) throw AccountFailure.invalidResponse;
      if (identity == null && identical(account, organization)) {
        // The organization's issuer binds the proven key on first issuance;
        // that binding is how a member's existing key joins the organization.
      } else if (identity == null) {
        // Issuance can create a binding. Reconnect must not silently give a
        // different, unbound hosted account this community's saved identity.
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
      // Issuers run in parallel, so a failure that started before another
      // relay's unresolved failure was recorded must not replace it.
      final otherScope =
          failure != null &&
          (failedRelay != config.baseUrl || failedPubkey != pubkey);
      if (!_disposed && !(otherScope && _failureSequence > started)) {
        _record(config.baseUrl, pubkey, error, started);
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
      _record(
        config.baseUrl,
        pubkeyFromNsec(config.nsec),
        AccountFailure.denied,
        ++_sequence,
      );
    }
  }

  void _record(
    String relay,
    String? pubkey,
    AccountFailure error,
    int sequence,
  ) {
    failedRelay = relay;
    failedPubkey = pubkey;
    failure = error;
    _failureSequence = sequence;
    notifyListeners();
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
  }) : _inner = inner ?? newAppHttpClient();
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
