import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'dart:typed_data';

import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:pointycastle/ecc/curves/secp256k1.dart';
import 'package:url_launcher/url_launcher.dart';

import 'event_signer.dart';

/// Non-secret release configuration; absence preserves OSS self-custody.
const enterpriseBuildConfig = String.fromEnvironment('BUZZ_BUILD_ENTERPRISE');
bool get enterpriseEnabled => enterpriseBuildConfig.isNotEmpty;

/// One app-owned identity, scoped to the fixed enterprise community. Never contains a Nostr private key.
class EnterpriseIdentity {
  EnterpriseIdentity({
    http.Client? client,
    FlutterSecureStorage? storage,
    String config = enterpriseBuildConfig,
    Stream<Uri>? callbacks,
    Future<bool> Function(Uri)? openBrowser,
  }) : _configuration = config,
       _client = client ?? http.Client(),
       _storage = storage ?? const FlutterSecureStorage(),
       _callbacks = callbacks,
       _openBrowser =
           openBrowser ??
           ((uri) => launchUrl(uri, mode: LaunchMode.externalApplication));
  static final instance = EnterpriseIdentity();
  final String _configuration;
  final http.Client _client;
  final FlutterSecureStorage _storage;
  final Stream<Uri>? _callbacks;
  final Future<bool> Function(Uri) _openBrowser;
  final revision = ValueNotifier<int>(0);
  Map<String, dynamic>? _tokens;
  Map<String, dynamic>? _identity;
  Future<void>? _refreshing;
  Future<void>? _restoring;
  bool _loggingIn = false;
  Completer<void>? _loginCancelled;
  int _generation = 0;
  bool get authenticated => _tokens != null && _identity != null;
  String? get pubkey => _identity?['pubkey'] as String?;
  String? get relayUrl => _identity?['relayHttpUrl'] as String?;
  Map<String, dynamic> get _config {
    final config = jsonDecode(_configuration) as Map<String, dynamic>;
    for (final key in ['signerUrl', 'issuer']) {
      final raw = config[key] as String;
      final url = Uri.parse(raw);
      if (url.scheme != 'https' ||
          url.host.isEmpty ||
          url.userInfo.isNotEmpty ||
          url.hasPort ||
          raw != raw.trim() ||
          !RegExp(r'^https://[^/@:?#]+(?:/[^?#]*)?$').hasMatch(raw) ||
          url.hasQuery ||
          url.hasFragment ||
          (key == 'issuer' && url.path != '' && url.path != '/') ||
          (key == 'signerUrl' &&
              (!url.path.endsWith('/') || url.path.contains('/v1/')))) {
        throw StateError('Invalid enterprise build endpoint');
      }
    }
    for (final key in [
      'clientId',
      'audience',
      'organization',
      'connection',
      'redirectUri',
    ]) {
      if (config[key] is! String || (config[key] as String).isEmpty) {
        throw StateError('Incomplete enterprise build configuration');
      }
    }
    if (config.length != 7 ||
        config['redirectUri'] != 'buzz://enterprise-login') {
      throw StateError('Invalid enterprise callback');
    }
    return config;
  }

  String _endpoint(String key, String path) =>
      '${(_config[key] as String).replaceFirst(RegExp(r'/+$'), '')}/$path';

  Future<Map<String, dynamic>> _post(
    String url,
    Map<String, dynamic> body, {
    String? token,
  }) async {
    final abort = Completer<void>();
    final request = http.AbortableRequest(
      'POST',
      Uri.parse(url),
      abortTrigger: abort.future,
    )..followRedirects = false;
    request.headers.addAll({
      'Content-Type': 'application/json',
      'Cache-Control': 'no-store',
      if (token != null) 'Authorization': 'Bearer $token',
    });
    request.body = jsonEncode(body);
    return (() async {
      final response = await _client.send(request);
      if (response.statusCode < 200 || response.statusCode >= 300) {
        await response.stream.listen((_) {}).cancel();
        throw StateError(
          'Corporate request rejected (${response.statusCode}); sign in again',
        );
      }
      final bytes = BytesBuilder(copy: false);
      await for (final chunk in response.stream) {
        if (bytes.length + chunk.length > 256 * 1024) {
          throw StateError('Corporate response too large');
        }
        bytes.add(chunk);
      }
      return jsonDecode(utf8.decode(bytes.takeBytes())) as Map<String, dynamic>;
    })().timeout(const Duration(seconds: 15)).whenComplete(() {
      if (!abort.isCompleted) abort.complete();
    });
  }

  void _validateTokens(Map<String, dynamic> tokens) {
    if (tokens['access_token'] is! String ||
        (tokens['access_token'] as String).isEmpty ||
        (tokens['access_token'] as String).length > 16384 ||
        RegExp(r'\s').hasMatch(tokens['access_token'] as String) ||
        tokens['token_type']?.toString().toLowerCase() != 'bearer' ||
        tokens['expires_in'] is! int ||
        tokens['expires_in'] < 1 ||
        tokens['expires_in'] > 300) {
      throw StateError('Invalid corporate credentials');
    }
    final refresh = tokens['refresh_token'];
    if (refresh != null &&
        (refresh is! String || refresh.isEmpty || refresh.length > 16384)) {
      throw StateError('Invalid corporate refresh credential');
    }
    tokens['expiresAt'] =
        DateTime.now().millisecondsSinceEpoch ~/ 1000 +
        (tokens['expires_in'] as int);
  }

  Map<String, dynamic> _validateIdentity(Map<String, dynamic> identity) {
    if (identity['pubkey'] is! String ||
        !RegExp(r'^[0-9a-f]{64}$').hasMatch(identity['pubkey'] as String)) {
      throw StateError('Invalid corporate identity');
    }
    // Nostr authors must be actual x-only secp256k1 points, not just hex.
    final publicKey = identity['pubkey'] as String;
    final point = ECCurve_secp256k1().curve.decodePoint(
      Uint8List.fromList([
        2,
        for (var i = 0; i < publicKey.length; i += 2)
          int.parse(publicKey.substring(i, i + 2), radix: 16),
      ]),
    );
    if (point == null || point.isInfinity) {
      throw StateError('Invalid corporate public key');
    }
    final rawRelay = identity['relayHttpUrl'] as String;
    if (RegExp(r'^https://[^/?#]*@').hasMatch(rawRelay)) {
      throw StateError('Invalid corporate community');
    }
    final relay = Uri.parse(rawRelay);
    if (relay.scheme != 'https' ||
        relay.host.isEmpty ||
        relay.userInfo.isNotEmpty ||
        relay.hasQuery ||
        relay.hasFragment ||
        (relay.path != '' && relay.path != '/') ||
        identity['relayWsUrl'] != 'wss://${relay.authority}') {
      throw StateError('Invalid corporate community');
    }
    return identity;
  }

  Future<void> restore() =>
      _restoring ??= _restore().whenComplete(() => _restoring = null);

  Future<void> _restore() async {
    if (_configuration.isEmpty || _identity != null || _loggingIn) return;
    _config; // Validate the real compiled configuration before touching credentials.
    final generation = _generation;
    await _storageTail;
    _checkGeneration(generation);
    final encoded = await _storage.read(key: 'buzz.enterprise.session');
    if (encoded == null || generation != _generation) return;
    final saved = jsonDecode(encoded) as Map<String, dynamic>;
    // Credentials never cross build-selected issuer/client/signer boundaries.
    if (saved['config'] != _configuration) {
      throw StateError('Corporate build changed; sign in again');
    }
    _identity = _validateIdentity(
      Map<String, dynamic>.from(saved['identity'] as Map),
    );
    _tokens = Map<String, dynamic>.from(saved['tokens'] as Map);
    if (_tokens?['access_token'] is! String ||
        (_tokens!['access_token'] as String).isEmpty ||
        (_tokens!['access_token'] as String).length > 16384 ||
        _tokens?['token_type']?.toString().toLowerCase() != 'bearer' ||
        _tokens?['expiresAt'] is! int) {
      _tokens = null;
      _identity = null;
      throw StateError('Invalid saved corporate credentials');
    }
    try {
      await accessToken();
      _checkGeneration(generation);
      revision.value++;
    } catch (_) {
      if (generation == _generation) {
        _identity = null;
        _tokens = null;
      }
      rethrow;
    }
  }

  // Serialize secure-storage mutations. A superseded login cannot overwrite a
  // newer login or recreate credentials after logout.
  Future<void> _storageTail = Future.value();
  Future<void> _storageMutation(Future<void> Function() action) {
    final operation = _storageTail.then((_) => action());
    _storageTail = operation.then((_) {}, onError: (Object _) {});
    return operation;
  }

  Future<void> _deleteSaved() =>
      _storageMutation(() => _storage.delete(key: 'buzz.enterprise.session'));

  Future<void> _persist(
    Map<String, dynamic> tokens,
    Map<String, dynamic> identity,
    int generation,
  ) => _storageMutation(() async {
    _checkGeneration(generation);
    await _storage.write(
      key: 'buzz.enterprise.session',
      value: jsonEncode({
        'config': _configuration,
        'tokens': tokens,
        'identity': identity,
      }),
    );
    if (generation != _generation) {
      await _storage.delete(key: 'buzz.enterprise.session');
      _checkGeneration(generation);
    }
  });

  Future<String> accessToken() async {
    final generation = _generation;
    final tokens = _tokens;
    if (tokens == null) throw StateError('Corporate login required');
    if ((tokens['expiresAt'] as int) <=
        DateTime.now().millisecondsSinceEpoch ~/ 1000 + 30) {
      _refreshing ??= _refresh().whenComplete(() => _refreshing = null);
      await _refreshing;
    }
    _checkGeneration(generation);
    return _tokens?['access_token'] as String? ??
        (throw StateError('Corporate login required'));
  }

  Future<void> _refresh() async {
    final generation = _generation;
    try {
      final refresh = _tokens?['refresh_token'] as String?;
      if (refresh == null) {
        throw StateError('Corporate session expired; sign in again');
      }
      await _deleteSaved();
      _checkGeneration(generation);
      final next = await _post(_endpoint('issuer', 'oauth/token'), {
        'grant_type': 'refresh_token',
        'client_id': _config['clientId'],
        'refresh_token': refresh,
      });
      _checkGeneration(generation);
      _validateTokens(next);
      final identity = _validateIdentity(
        await _post(
          _endpoint('signerUrl', 'v1/buzz/identity/ensure'),
          {},
          token: next['access_token'] as String,
        ),
      );
      if (generation != _generation ||
          identity['pubkey'] != pubkey ||
          identity['relayHttpUrl'] != relayUrl) {
        throw StateError('Corporate account changed');
      }
      await _persist(next, identity, generation);
      if (generation != _generation) {
        throw StateError('Corporate account changed');
      }
      _tokens = next;
    } catch (_) {
      if (generation == _generation) {
        _tokens = null;
        revision.value++;
        await _storageMutation(() async {
          _checkGeneration(generation);
          await _storage.delete(key: 'buzz.enterprise.session');
        });
      }
      rethrow; // Never retry a potentially consumed rotating refresh token automatically.
    }
  }

  Future<void> login() async {
    if (_loggingIn) throw StateError('Corporate login already in progress');
    _loggingIn = true;
    final cancelled = Completer<void>();
    _loginCancelled = cancelled;
    final generation = ++_generation;
    _tokens = null;
    _identity = null;
    revision.value++;
    // Drain any rotating refresh before a new login can replace the same secure-storage record.
    try {
      await _refreshing;
    } catch (_) {
      /* A new browser login is the recovery path. */
    }
    StreamSubscription<Uri>? subscription;
    try {
      _checkGeneration(generation);
      await _deleteSaved();
      _checkGeneration(generation);
      final config = _config;
      final random = Random.secure();
      String randomToken() => base64Url
          .encode(List.generate(32, (_) => random.nextInt(256)))
          .replaceAll('=', '');
      final verifier = randomToken(), state = randomToken();
      final challenge = base64Url
          .encode(
            SHA256Digest().process(Uint8List.fromList(ascii.encode(verifier))),
          )
          .replaceAll('=', '');
      final redirect = Uri.parse(config['redirectUri'] as String);
      final callback = Completer<Uri>();
      subscription = (_callbacks ?? AppLinks().uriLinkStream).listen((uri) {
        if (uri.scheme != redirect.scheme ||
            uri.authority != redirect.authority ||
            uri.path != redirect.path ||
            uri.hasFragment) {
          return;
        }
        if (uri.queryParametersAll['state']?.length != 1 ||
            uri.queryParameters['state'] != state ||
            uri.queryParametersAll['code']?.length != 1 ||
            uri.queryParameters.containsKey('error')) {
          return;
        }
        if (!callback.isCompleted) callback.complete(uri);
      });
      final authorize = Uri.parse(_endpoint('issuer', 'authorize')).replace(
        queryParameters: {
          'response_type': 'code',
          'client_id': config['clientId'],
          'redirect_uri': redirect.toString(),
          'audience': config['audience'],
          'organization': config['organization'],
          'connection': config['connection'],
          'scope': 'openid profile email offline_access buzz:sign',
          'state': state,
          'code_challenge_method': 'S256',
          'code_challenge': challenge,
        },
      );
      if (!await _openBrowser(authorize)) {
        throw StateError('Cannot open corporate login');
      }
      _checkGeneration(generation);
      final uri = await Future.any<Uri>([
        callback.future,
        cancelled.future.then(
          (_) => throw StateError('Corporate login cancelled'),
        ),
      ]).timeout(const Duration(minutes: 5));
      _checkGeneration(generation);
      final tokens = await _post(_endpoint('issuer', 'oauth/token'), {
        'grant_type': 'authorization_code',
        'client_id': config['clientId'],
        'redirect_uri': redirect.toString(),
        'code': uri.queryParameters['code'],
        'code_verifier': verifier,
      });
      _checkGeneration(generation);
      _validateTokens(tokens);
      final identity = _validateIdentity(
        await _post(
          _endpoint('signerUrl', 'v1/buzz/identity/ensure'),
          {},
          token: tokens['access_token'] as String,
        ),
      );
      if (generation != _generation) {
        throw StateError('Corporate login changed');
      }
      await _persist(tokens, identity, generation);
      _checkGeneration(generation);
      _tokens = tokens;
      _identity = identity;
      revision.value++;
    } catch (_) {
      if (generation == _generation) {
        await _storageMutation(() async {
          _checkGeneration(generation);
          await _storage.delete(key: 'buzz.enterprise.session');
        });
      }
      rethrow;
    } finally {
      await subscription?.cancel();
      if (identical(_loginCancelled, cancelled)) _loginCancelled = null;
      _loggingIn = false;
    }
  }

  Future<void> logout() async {
    _generation++;
    final cancelled = _loginCancelled;
    if (cancelled != null && !cancelled.isCompleted) cancelled.complete();
    _tokens = null;
    _identity = null;
    revision.value++;
    await _deleteSaved();
  }

  /// Capture authority for one login and its server-selected community.
  RemoteEventSigner captureSigner() {
    if (!authenticated) throw StateError('Corporate login required');
    return RemoteEventSigner._(this, _generation, pubkey!, relayUrl!);
  }

  void _checkGeneration(int generation) {
    if (generation != _generation) {
      throw StateError('Corporate account changed');
    }
  }

  Future<nostr.Event> _sign(UnsignedEvent unsigned, int generation) async {
    _checkGeneration(generation);
    final expected = pubkey;
    if (!authenticated || expected != unsigned.publicKey) {
      throw StateError('Corporate signing identity mismatch');
    }
    final kind = unsigned.kind;
    final content = unsigned.content;
    final tags = unsigned.tags;
    final template = {
      'kind': kind,
      'content': content,
      'tags': tags.map((t) => List<String>.of(t)).toList(),
      'created_at': unsigned.createdAt,
    };
    final purpose = kind == 22242
        ? 'nip42-auth'
        : kind == 27235
        ? 'http-auth'
        : kind == 24242
        ? (tags.any((t) => t.length > 1 && t[0] == 't' && t[1] == 'upload')
              ? 'media-upload'
              : 'media-read')
        : 'publish';
    final token = await accessToken();
    _checkGeneration(generation);
    final response = await _post(
      _endpoint('signerUrl', 'v1/buzz/identity/sign'),
      {'purpose': purpose, 'event': template},
      token: token,
    );
    final event = nostr.Event.fromJson(
      jsonEncode(response['event']),
    ); // Constructor verifies id and Schnorr signature.
    if (generation != _generation ||
        event.pubkey != expected ||
        event.kind != kind ||
        event.content != content ||
        event.createdAt != template['created_at'] ||
        jsonEncode(event.tags) != jsonEncode(template['tags'])) {
      throw StateError('Corporate signature or identity mismatch');
    }
    return event;
  }
}

/// Signing capability captured from an authenticated login, never a key source.
/// It cannot move to a new login, even if that login returns the same pubkey.
final class RemoteEventSigner implements EventSigner {
  RemoteEventSigner._(
    this._owner,
    this._generation,
    this.publicKey,
    this.relayUrl,
  );
  final EnterpriseIdentity _owner;
  final int _generation;
  @override
  final String publicKey;
  final String relayUrl;

  /// Fail before transport if this capability has been revoked or superseded.
  void checkCurrent() {
    _owner._checkGeneration(_generation);
    if (!_owner.authenticated) throw StateError('Corporate login required');
  }

  @override
  Future<nostr.Event> sign(UnsignedEvent event) {
    checkCurrent();
    return _owner._sign(event, _generation);
  }
}
