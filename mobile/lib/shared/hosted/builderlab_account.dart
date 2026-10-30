import '../storage/serial_queue.dart';
import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'package:http/http.dart' as http;

import '../storage/secure_storage.dart';

import 'hosted_environment.dart';
import 'bounded_http_response.dart';
import '../relay/relay_session.dart';
import 'account_identity_store.dart';

/// Errors contain no response bodies, credentials, or recovered private keys.
/// Only [AccountFailure.signedOut] means "no session"; every other failure
/// leaves the account state unknown.
enum AccountFailure {
  signedOut,
  denied,
  accountSetupRequired,
  unavailable,
  invalidResponse,
  storage,

  /// Login completed for a different account than the stored session.
  differentAccount,

  /// The server definitively refused this request (400/404/409/422); sending
  /// it again unchanged cannot succeed.
  rejected,
}

class BuilderlabAccount {
  const BuilderlabAccount({
    required this.subject,
    required this.expiresAt,
    this.needsAccountSetup = false,
  });
  final bool needsAccountSetup;
  final String subject;
  final DateTime expiresAt;
}

/// Builderlab's CLI and NIP-FA adapter use the same server-side CLI credential.
/// This client uses the native header, never a browser cookie. Browser handoff
/// is deliberately a separate boundary: an app link is not an account session.
///
/// One session per device, scoped to its account subject. A login for another
/// subject is refused until the caller clears the session and every fact it
/// holds for the previous subject.
class BuilderlabAccountClient {
  BuilderlabAccountClient({
    http.Client? httpClient,
    FlutterSecureStorage? storage,
  }) : _http = httpClient ?? http.Client(),
       _storage = storage ?? defaultStorage;

  /// Device-only: the session never syncs or restores onto another device.
  @visibleForTesting
  static const defaultStorage = accountSecureStorage;

  static final origin = HostedEnvironment.origin;
  static const _prefix = '/api/goose';

  /// Remote-agent registry routes, shared with desktop.
  /// Safe request diagnostics: no headers, credentials, bodies or user data.
  ({String path, int status})? lastRejectedRequest;

  static const _beekeeperRoutes = {
    '/v3/beekeeper/get-model-catalog',
    '/v3/beekeeper/register-agent',
    '/v3/beekeeper/update-agent',
    '/v3/beekeeper/delete-agent',
    '/v3/beekeeper/attest-agent',
    '/v3/beekeeper/list-agents',
  };

  /// Sessions are scoped per origin so staging never reads production state.
  static final _storageKey =
      'buzz_hosted_session_${origin.host.replaceAll('.', '_')}_v1';
  final http.Client _http;
  final FlutterSecureStorage _storage;
  String? _credential;
  BuilderlabAccount? _account;
  final _queue = SerialQueue();
  bool _disposed = false;

  BuilderlabAccount? get account => _account;

  /// The signed-in subject, notifying when an account signs in, out or changes.
  ValueListenable<String?> get subjectChanges => _subject;
  final _subject = ValueNotifier<String?>(null);

  /// Runs operations one at a time in call order, so a callback exchange that
  /// arrives during startup restore waits instead of failing.
  Future<T> _serial<T>(Future<T> Function() operation) {
    final result = _queue.run(() {
      if (_disposed) throw AccountFailure.unavailable;
      return operation();
    });
    return result;
  }

  Future<BuilderlabAccount?> restore() => _serial(() async {
    final record = await _readRecord();
    if (record == null) return null;
    try {
      final verified = await _verify(record.credential);
      if (verified.subject != record.subject) {
        throw AccountFailure.invalidResponse;
      }
      return _adopt(record.credential, verified);
    } on AccountFailure catch (failure) {
      if (failure == AccountFailure.signedOut) {
        await _clear();
        return null;
      }
      rethrow;
    }
  });

  /// Receives a one-use code only from a validated browser callback. PKCE is
  /// verified by Builderlab; state and callback origin are checked by the caller.
  Future<BuilderlabAccount> exchange({
    required String code,
    required String verifier,
  }) => _serial(() async {
    if (code.isEmpty ||
        !RegExp(r'^[A-Za-z0-9._~-]{43,128}$').hasMatch(verifier)) {
      throw AccountFailure.invalidResponse;
    }
    final stored = await _readRecord();
    final result = await _request(
      '/v1/auth/login/exchange',
      body: {'code': code, 'code_verifier': verifier},
    );
    final credential = result['session_credential'];
    if (credential is! String ||
        credential.isEmpty ||
        credential.length > 512) {
      throw AccountFailure.invalidResponse;
    }
    final verified = await _verify(credential);
    if (DateTime.tryParse(
          result['expires_at'] is String ? result['expires_at'] as String : '',
        ) !=
        verified.expiresAt) {
      throw AccountFailure.invalidResponse;
    }
    if (stored != null && stored.subject != verified.subject) {
      throw AccountFailure.differentAccount;
    }
    if (_disposed) throw AccountFailure.unavailable;
    final record = jsonEncode({
      'origin': origin.toString(),
      'credential': credential,
      'subject': verified.subject,
      'expires_at': verified.expiresAt.toIso8601String(),
    });
    try {
      await _storage.write(key: _storageKey, value: record);
      if (await _storage.read(key: _storageKey) != record) {
        throw AccountFailure.storage;
      }
    } catch (_) {
      throw AccountFailure.storage;
    }
    return _adopt(credential, verified);
  });

  /// An unreadable record is a storage failure, never an absent session: its
  /// subject is unknown, so only an explicit [clearLocalSession] may drop it.
  Future<({String credential, String subject})?> _readRecord() async {
    final String? saved;
    try {
      saved = await _storage.read(key: _storageKey);
    } catch (_) {
      throw AccountFailure.storage;
    }
    if (saved == null) return null;
    try {
      final value = jsonDecode(saved);
      if (value is Map<String, dynamic> &&
          value['origin'] == origin.toString() &&
          value['credential'] is String &&
          value['subject'] is String) {
        return (
          credential: value['credential'] as String,
          subject: value['subject'] as String,
        );
      }
    } on FormatException {
      // Falls through to the storage failure below.
    }
    throw AccountFailure.storage;
  }

  BuilderlabAccount _adopt(String credential, BuilderlabAccount verified) {
    if (_disposed) throw AccountFailure.unavailable;
    _credential = credential;
    _subject.value = verified.subject;
    return _account = verified;
  }

  Future<BuilderlabAccount> _verify(String credential) async {
    final result = await _request('/v1/auth/me', credential: credential);
    final subject = result['subject'];
    final expiry = result['expires_at'];
    final parsed = expiry is String ? DateTime.tryParse(expiry) : null;
    if (subject is! String || subject.isEmpty || parsed == null) {
      throw AccountFailure.invalidResponse;
    }
    if (!parsed.isAfter(DateTime.now().toUtc())) throw AccountFailure.signedOut;
    final standing = result['account_standing'];
    final needsSetup =
        standing is Map &&
        (standing['details_status'] == 'ACCOUNT_DETAILS_STATUS_MISSING' ||
            standing['screening_status'] ==
                'ACCOUNT_SCREENING_STATUS_NOT_SCREENED');
    return BuilderlabAccount(
      subject: subject,
      expiresAt: parsed,
      needsAccountSetup: needsSetup,
    );
  }

  /// Public identity metadata only. Private-key recovery belongs to the identity
  /// storage owner, which must derive and verify the key before any persistence.
  Future<Map<String, dynamic>> currentIdentity() =>
      post('/v1/buzz/nostr-identities/current', {});

  /// Saves a successfully admitted destination to the same account, even if
  /// another session operation was queued before this request runs.
  Future<void> rememberCommunity(
    String relay, {
    required String expectedSubject,
  }) async {
    final result = await _post('/v1/buzz/known-communities/add', {
      'community_url': relay,
    }, expectedSubject: expectedSubject);
    if (result['error'] != null) throw AccountFailure.rejected;
  }

  /// Authenticated JSON POST to a Buzz account route on the fixed origin. The
  /// credential never leaves this client. A 401 clears the stored session only
  /// when the session check also rejects it; otherwise it is [AccountFailure.denied].
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) => _post(path, body, expectedSubject: expectedSubject);

  Future<Map<String, dynamic>> _post(
    String path,
    Map<String, dynamic> body, {
    String? signingKey,
    String? expectedSubject,
  }) {
    if (!RegExp(r'^/v1/buzz(/[a-z0-9_-]+)+$').hasMatch(path) &&
        !_beekeeperRoutes.contains(path)) {
      throw ArgumentError.value(
        path,
        'path',
        'not an allowed Builderlab route',
      );
    }
    return _serial(() async {
      final credential = _credential;
      if (credential == null) throw AccountFailure.signedOut;
      if (expectedSubject != null && _account?.subject != expectedSubject) {
        throw AccountFailure.denied;
      }
      try {
        return await _request(
          path,
          credential: credential,
          body: body,
          signingKey: signingKey,
        );
      } on AccountFailure catch (failure) {
        if (failure == AccountFailure.denied &&
            _account?.needsAccountSetup == true) {
          throw AccountFailure.accountSetupRequired;
        }
        if (failure != AccountFailure.signedOut) rethrow;
      }
      // Routes also answer 401 for authorization failures, so only a rejected
      // session check ends the session.
      try {
        await _verify(credential);
      } on AccountFailure catch (failure) {
        if (failure == AccountFailure.signedOut) await _clear();
        rethrow;
      }
      throw AccountFailure.denied;
    });
  }

  /// Proves the saved key. The issuer may establish its first account binding;
  /// conflicting or retired bindings are refused atomically by the backend.
  Future<Map<String, dynamic>> relayAssertion(
    String relay,
    String nsec, {
    required String expectedSubject,
  }) => _post(
    '/v1/buzz/identity/assertions',
    {
      'relay_url': relay,
      'nostr_pubkey': AccountIdentityStore.publicKeyForNsec(nsec),
    },
    signingKey: nsec,
    expectedSubject: expectedSubject,
  );

  Future<void> clearLocalSession() => _serial(_clear);
  Future<void> _clear() async {
    _credential = null;
    _account = null;
    if (!_disposed) _subject.value = null;
    try {
      await _storage.delete(key: _storageKey);
    } catch (_) {
      throw AccountFailure.storage;
    }
  }

  Future<Map<String, dynamic>> _request(
    String path, {
    String? credential,
    Map<String, dynamic>? body,
    String? signingKey,
  }) async {
    try {
      final request =
          http.Request(
              body == null ? 'GET' : 'POST',
              origin.replace(path: '$_prefix$path'),
            )
            ..followRedirects = false
            ..headers['Accept'] = 'application/json';
      if (credential != null) {
        request.headers['X-BB-Session-Credential'] = credential;
      }
      if (body != null) {
        request.headers['Content-Type'] = 'application/json';
        request.headers['Origin'] = origin.toString();
        request.body = jsonEncode(body);
        if (signingKey != null) {
          request.headers['Authorization'] = buildNip98AuthHeader(
            method: 'POST',
            url: request.url.toString(),
            bodyBytes: request.bodyBytes,
            nsec: signingKey,
          );
        }
      }
      final response = await boundedHttpResponse(_http, request);
      if (kDebugMode && response.statusCode >= 400) {
        debugPrint(
          'Builderlab ${origin.host} ${request.method} $path: HTTP ${response.statusCode} contentType=${response.headers['content-type']?.split(';').first}',
        );
      }
      if (response.statusCode == 401) throw AccountFailure.signedOut;
      if (response.statusCode == 403) throw AccountFailure.denied;
      if (const {400, 404, 409, 422}.contains(response.statusCode)) {
        lastRejectedRequest = (path: path, status: response.statusCode);
        throw AccountFailure.rejected;
      }
      if (response.statusCode < 200 || response.statusCode >= 300) {
        throw AccountFailure.unavailable;
      }
      final value = jsonDecode(utf8.decode(response.bodyBytes));
      if (value is! Map<String, dynamic>) throw AccountFailure.invalidResponse;
      return value;
    } on AccountFailure {
      rethrow;
    } on FormatException {
      throw AccountFailure.invalidResponse;
    } catch (_) {
      throw AccountFailure.unavailable;
    }
  }

  void dispose() {
    _disposed = true;
    _credential = null;
    _account = null;
    _subject.dispose();
    _http.close();
  }
}
