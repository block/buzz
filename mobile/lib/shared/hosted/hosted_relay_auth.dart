import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../relay/relay_provider.dart';
import 'builderlab_account.dart';
import 'account_identity_store.dart';
import 'hosted_environment.dart';
import 'nip_fi_discovery.dart';
import 'organization_environment.dart';
import 'organization_key_backup.dart';
import '../push/push_bridge.dart';

final builderlabAccountProvider = Provider<BuilderlabAccountClient>((ref) {
  final account = BuilderlabAccountClient(
    // Hosted communities that enforce NIP-FI need the extension to mint too.
    // A build without hosted relay auth still clears a copy an earlier build
    // shared, so the extension cannot keep minting after sign-out.
    onSessionChanged: (credential) => shareHostedSessionWithNotifications(
      origin: HostedEnvironment.origin,
      credential: HostedRelayAuth.configured && HostedEnvironment.enabled
          ? credential
          : null,
      relayDomains: HostedEnvironment.authDomains,
      communityDomain: HostedEnvironment.communitySuffix,
    ),
  );
  ref.onDispose(account.dispose);
  return account;
});

/// The organization's own account session, separate from the hosted account.
/// Null unless this build configures [OrganizationEnvironment].
final organizationAccountProvider = Provider<BuilderlabAccountClient?>((ref) {
  final origin = OrganizationEnvironment.origin;
  if (origin == null) return null;
  final account = BuilderlabAccountClient(
    accountOrigin: origin,
    // The notification extension mints its own assertions for these relays.
    onSessionChanged: (credential) => shareOrganizationSessionWithNotifications(
      origin: origin,
      credential: credential,
      relayHosts: OrganizationEnvironment.relayHosts,
    ),
  );
  ref.onDispose(account.dispose);
  return account;
});

/// Outlives relay-auth rebuilds, so its settled keys and in-flight attempts
/// are never duplicated. Null unless this build configures an organization.
final organizationKeyBackupProvider = Provider<OrganizationKeyBackup?>((ref) {
  final organization = ref.watch(organizationAccountProvider);
  return organization == null ? null : OrganizationKeyBackup(organization);
});

final hostedRelayAuthProvider = Provider<HostedRelayAuth>((ref) {
  final auth = HostedRelayAuth(
    ref.watch(builderlabAccountProvider),
    organization: ref.watch(organizationAccountProvider),
    organizationBackup: ref.watch(organizationKeyBackupProvider),
  );
  ref.onDispose(auth.dispose);
  return auth;
});

/// Each transport owns its client lifetime; configuration and auth are shared.
final hostedHttpClientFactoryProvider = Provider<http.Client Function()>((ref) {
  final auth = ref.watch(hostedRelayAuthProvider);
  final config = ref.watch(relayConfigProvider);
  return () => HostedRelayHttpClient(auth, config);
});

/// The build's one Builderlab session, with a short-lived assertion for the
/// current relay/key. A public build's hosted communities use [account] when
/// [enabled] and their relay's NIP-FI discovery requires evidence; an internal
/// build's organization relays always use [organization]. Enable alongside
/// backend issuance; self-hosted communities stay usable.
class HostedRelayAuth extends ChangeNotifier {
  HostedRelayAuth(
    this.account, {
    this.organization,
    this.organizationBackup,
    this.organizationRelay = OrganizationEnvironment.protects,
    this.enabled = configured,
    DateTime Function()? now,
    NipFiRequirementFetch? discover,
  }) : _now = now ?? DateTime.now,
       _discover = discover ?? _fetchRequirement;

  static Future<bool> _fetchRequirement(Uri origin) async {
    final client = http.Client();
    try {
      return await fetchNipFiRequirement(client, origin);
    } finally {
      client.close();
    }
  }

  /// Whether this build turns hosted relay auth on.
  static const configured = bool.fromEnvironment('BUZZ_HOSTED_RELAY_AUTH');

  final BuilderlabAccountClient account;
  final BuilderlabAccountClient? organization;

  /// Backs up each organization relay key once its assertion has bound it.
  final OrganizationKeyBackup? organizationBackup;
  final bool Function(Uri relay) organizationRelay;
  final bool enabled;
  final DateTime Function() _now;
  final NipFiRequirementFetch _discover;
  // NIP-FI discovery per hosted relay origin: the read in flight, shared by
  // concurrent callers, and the last result. The issuer refuses hosts that have
  // not enabled NIP-FI, so those connect without evidence.
  final _discovering = <String, Future<bool>>{};
  final _required = <String, bool>{};
  // One assertion per relay and key, so issuing for an inactive community (for
  // example a background revocation) never evicts the active session's
  // credentials that native media paths read synchronously.
  final _cached =
      <
        ({String relay, String pubkey}),
        ({String token, DateTime expiry, BuilderlabAccount account})
      >{};
  // Per account client, so a stalled issuer never delays an unrelated one.
  // Waiters share account restoration, but only a recorded issuance for their
  // own relay and key can fail on their behalf.
  final _pending =
      <
        BuilderlabAccountClient,
        ({Future<void> operation, String relay, String pubkey, bool background})
      >{};
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
    final client = _accountFor(config);
    if (client == null) return const {};
    if (_hostedOrigin(config, client) case final origin?
        when _required[origin] == false) {
      return const {};
    }
    final pubkey = pubkeyFromNsec(config.nsec);
    if (pubkey == null) return null;
    final cached =
        _cached[(
          relay: Uri.parse(config.wsUrl).replace(path: '').toString(),
          pubkey: pubkey,
        )];
    if (cached == null ||
        !identical(cached.account, client.account) ||
        !cached.expiry.isAfter(_now())) {
      return null;
    }
    return {'Nostr-Federated-Identity': 'Bearer ${cached.token}'};
  }

  /// [headers] for a request made outside [HostedRelayHttpClient], such as a
  /// native download. Scoped like that client: only the relay's own origin
  /// gets the assertion, so it never reaches a third-party host.
  Future<Map<String, String>> headersForUrl(RelayConfig config, Uri url) async {
    if (!_isRelayOrigin(config, url)) return const {};
    return headers(config);
  }

  /// Synchronous [headersForUrl] for callers that hand headers to native code
  /// up front. Uses the assertion the live relay session already holds; with
  /// none cached the request goes out without it and the relay decides.
  Map<String, String> cachedHeadersForUrl(RelayConfig config, Uri url) {
    if (!_isRelayOrigin(config, url)) return const {};
    return cachedHeaders(config) ?? const {};
  }

  static bool _isRelayOrigin(RelayConfig config, Uri url) =>
      (url.scheme == 'http' || url.scheme == 'https') &&
      // `Uri.origin` throws for a hostless URI, which a community can supply.
      url.host.isNotEmpty &&
      url.origin == Uri.parse(config.baseUrl).origin;

  /// The assertion for [config]'s relay and key. A [background] caller (such
  /// as a push revocation for a removed community) never records its failure
  /// or success, so it cannot hide or clear the active session's recovery.
  /// It never backs up the key or binds it to an unbound account, and its
  /// assertion bypasses the shared cache: each attempt gets a fresh one, and
  /// foreground use never inherits an assertion whose key was not backed up.
  /// Without an assertion it gets no headers rather than a failure, so the
  /// relay decides.
  ///
  /// A hosted relay gets an assertion only when its NIP-FI discovery requires
  /// one. Every caller opening a new socket passes [fresh] to reread
  /// discovery, so a community that changes enforcement is picked up on its
  /// next connect; HTTP requests reuse the latest discovery.
  Future<Map<String, String>> headers(
    RelayConfig config, {
    bool background = false,
    bool fresh = false,
  }) async {
    final client = _accountFor(config);
    if (client == null) return const {};
    try {
      if (_hostedOrigin(config, client) case final origin?
          when !await _requires(origin, fresh)) {
        return const {};
      }
      return await _headers(config, client, background);
    } on AccountFailure catch (failure) {
      // Cleanup still reaches a relay that does not enforce assertions; an
      // enforcing relay refuses, and the caller's durable retry tries again.
      if (background) return const {};
      // Without an organization session, connect as before and let the relay
      // decide: an enforcing relay refuses, and sign-in is offered.
      if (failure == AccountFailure.signedOut &&
          identical(client, organization)) {
        return const {};
      }
      rethrow;
    }
  }

  /// The relay origin whose discovery decides, or null for organization relays,
  /// which always use their own issuer.
  String? _hostedOrigin(RelayConfig config, BuilderlabAccountClient client) =>
      identical(client, organization) ? null : Uri.parse(config.baseUrl).origin;

  Future<bool> _requires(String origin, bool fresh) async {
    final known = _required[origin];
    if (!fresh && known != null) return known;
    final pending = _discovering[origin] ??= _discover(Uri.parse(origin));
    try {
      return _required[origin] = await pending;
    } on Exception catch (error) {
      // The last answer stays for requests. Without one, or for a new
      // connection, retry with backoff rather than guessing.
      debugPrint('NIP-FI discovery failed for $origin: $error');
      throw AccountFailure.unavailable;
    } finally {
      if (identical(_discovering[origin], pending)) {
        _discovering.remove(origin);
      }
    }
  }

  Future<Map<String, String>> _headers(
    RelayConfig config,
    BuilderlabAccountClient client,
    bool background,
  ) async {
    final relay = Uri.parse(config.wsUrl).replace(path: '').toString();
    final pubkey = pubkeyFromNsec(config.nsec);
    if (pubkey == null) throw AccountFailure.denied;
    while (!_disposed) {
      final cached = background
          ? null
          : _cached[(relay: relay, pubkey: pubkey)];
      if (cached != null &&
          identical(cached.account, client.account) &&
          cached.expiry.isAfter(_now())) {
        return {'Nostr-Federated-Identity': 'Bearer ${cached.token}'};
      }
      final pending = _pending[client];
      if (pending != null) {
        // Different communities share account restoration, never assertions.
        // A background failure is never recorded, so it is never inherited.
        try {
          await pending.operation;
        } on AccountFailure {
          if (!pending.background &&
              pending.relay == relay &&
              pending.pubkey == pubkey &&
              failedRelay == config.baseUrl &&
              failedPubkey == pubkey) {
            rethrow;
          }
        }
        continue;
      }
      final operation = _issue(config, client, relay, pubkey, background);
      _pending[client] = (
        operation: operation,
        relay: relay,
        pubkey: pubkey,
        background: background,
      );
      try {
        final token = await operation;
        if (token != null) {
          return {'Nostr-Federated-Identity': 'Bearer $token'};
        }
      } finally {
        if (identical(_pending[client]?.operation, operation)) {
          _pending.remove(client);
        }
      }
    }
    throw AccountFailure.unavailable;
  }

  /// The issued token, or null when the account session changed meanwhile.
  Future<String?> _issue(
    RelayConfig config,
    BuilderlabAccountClient account,
    String relay,
    String pubkey,
    bool background,
  ) async {
    final started = ++_sequence;
    try {
      if (account.account == null && await account.restore() == null) {
        // A push stop pending at sign-out still gets a few attempts.
        if (!background) throw AccountFailure.signedOut;
        final value = await account.pushStopAssertion(relay, config.nsec!);
        if (_disposed) throw AccountFailure.unavailable;
        return _token(value, pubkey).token;
      }
      final session = account.account!;
      final current = await account.currentIdentity();
      if (!identical(session, account.account)) throw AccountFailure.denied;
      final identity = current['identity'];
      if (current['error'] != null) throw AccountFailure.invalidResponse;
      if (identity == null && background) {
        // Issuance can create a binding, and a background caller's key may be
        // one the user discarded. Only foreground use chooses the identity.
        throw AccountFailure.denied;
      } else if (identity == null && identical(account, organization)) {
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
      final (:token, :expiry) = _token(value, pubkey);
      final now = _now().millisecondsSinceEpoch ~/ 1000;
      final expires = DateTime.fromMillisecondsSinceEpoch(
        expiry * 1000,
        isUtc: true,
      );
      if (_disposed) throw AccountFailure.unavailable;
      if (!identical(session, account.account)) {
        return null;
      }
      // A background caller may be removing this community: it must not clear
      // recovery, back up a discarded key or seed the shared cache.
      if (background) return token;
      _cached.removeWhere((_, entry) => !entry.expiry.isAfter(_now()));
      _cached[(relay: relay, pubkey: pubkey)] = (
        token: token,
        expiry: expires.subtract(
          Duration(seconds: (expiry - now) > 60 ? 30 : (expiry - now) ~/ 2),
        ),
        account: session,
      );
      // Issuance alone cannot prove admission after a relay refusal.
      if (failure != AccountFailure.denied) connected(config);
      if (identical(account, organization)) {
        unawaited(organizationBackup?.afterAssertion(config.nsec!, pubkey));
      }
      return token;
    } on AccountFailure catch (error) {
      // Issuers run in parallel, so a failure that started before another
      // relay's unresolved failure was recorded must not replace it.
      final otherScope =
          failure != null &&
          (failedRelay != config.baseUrl || failedPubkey != pubkey);
      if (!background &&
          !_disposed &&
          !(otherScope && _failureSequence > started)) {
        _record(config.baseUrl, pubkey, error, started);
      }
      rethrow;
    }
  }

  /// An issued assertion for [pubkey], with its expiry in Unix seconds.
  ({String token, int expiry}) _token(
    Map<String, dynamic> value,
    String pubkey,
  ) {
    final token = value['assertion'];
    final expiry = value['expires_at'];
    if (token is! String ||
        token.isEmpty ||
        token.length > 16384 ||
        token.contains(RegExp(r'[\r\n]')) ||
        value['nostr_pubkey'] != pubkey ||
        expiry is! int ||
        expiry <= _now().millisecondsSinceEpoch ~/ 1000) {
      throw AccountFailure.invalidResponse;
    }
    return (token: token, expiry: expiry);
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
    _cached.removeWhere((_, entry) => 'Bearer ${entry.token}' == usedToken);
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
    _cached.clear();
    _discovering.clear();
    _required.clear();
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
