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
import 'package:url_launcher/url_launcher.dart';

/// Non-secret release configuration; absence preserves OSS self-custody.
const enterpriseBuildConfig = String.fromEnvironment('BUZZ_BUILD_ENTERPRISE');
bool get enterpriseEnabled => enterpriseBuildConfig.isNotEmpty;

/// One app-owned identity, scoped to the fixed enterprise community. Never contains a Nostr private key.
class EnterpriseIdentity {
  EnterpriseIdentity({
    http.Client? client,
    FlutterSecureStorage? storage,
    String config = enterpriseBuildConfig,
  }) : _configuration = config,
       _client = client ?? http.Client(),
       _storage = storage ?? const FlutterSecureStorage();
  static final instance = EnterpriseIdentity();
  final String _configuration;
  final http.Client _client;
  final FlutterSecureStorage _storage;
  final revision = ValueNotifier<int>(0);
  Map<String, dynamic>? _tokens;
  Map<String, dynamic>? _identity;
  Future<void>? _refreshing;
  Future<void>? _restoring;
  bool _loggingIn = false;
  int _generation = 0;
  bool get authenticated => _tokens != null && _identity != null;
  String? get pubkey => _identity?['pubkey'] as String?;
  String? get relayUrl => _identity?['relayHttpUrl'] as String?;
  Map<String, dynamic> get _config {
    final config = jsonDecode(_configuration) as Map<String, dynamic>;
    for (final key in ['signerUrl', 'issuer']) {
      final url = Uri.parse(config[key] as String);
      if (url.scheme != 'https' ||
          url.host.isEmpty ||
          url.userInfo.isNotEmpty ||
          url.hasQuery ||
          url.hasFragment ||
          (key == 'issuer' && url.path != '' && url.path != '/')) {
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
    final redirect = Uri.parse(config['redirectUri'] as String);
    if (redirect.scheme != 'buzz' ||
        redirect.host != 'enterprise-login' ||
        redirect.hasQuery ||
        redirect.hasFragment) {
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
    final relay = Uri.parse(identity['relayHttpUrl'] as String);
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
    if (_configuration.isEmpty || _identity != null) return;
    final generation = _generation;
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
    if (_tokens?['access_token'] is! String || _tokens?['expiresAt'] is! int) {
      _tokens = null;
      _identity = null;
      throw StateError('Invalid saved corporate credentials');
    }
    try {
      await accessToken();
    } catch (_) {
      _identity = null;
      _tokens = null;
      rethrow;
    }
  }

  Future<void> _persist(
    Map<String, dynamic> tokens,
    Map<String, dynamic> identity,
  ) => _storage.write(
    key: 'buzz.enterprise.session',
    value: jsonEncode({
      'config': _configuration,
      'tokens': tokens,
      'identity': identity,
    }),
  );

  Future<String> accessToken() async {
    final tokens = _tokens;
    if (tokens == null) throw StateError('Corporate login required');
    if ((tokens['expiresAt'] as int) <=
        DateTime.now().millisecondsSinceEpoch ~/ 1000 + 30) {
      _refreshing ??= _refresh().whenComplete(() => _refreshing = null);
      await _refreshing;
    }
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
      await _storage.delete(key: 'buzz.enterprise.session');
      final next = await _post(_endpoint('issuer', 'oauth/token'), {
        'grant_type': 'refresh_token',
        'client_id': _config['clientId'],
        'refresh_token': refresh,
      });
      _validateTokens(next);
      final identity = _validateIdentity(
        await _post(
          _endpoint('signerUrl', 'v1/buzz/enterprise-signer/session'),
          {},
          token: next['access_token'] as String,
        ),
      );
      if (generation != _generation ||
          identity['pubkey'] != pubkey ||
          identity['relayHttpUrl'] != relayUrl) {
        throw StateError('Corporate account changed');
      }
      await _persist(next, identity);
      if (generation != _generation) {
        throw StateError('Corporate account changed');
      }
      _tokens = next;
    } catch (_) {
      if (generation == _generation) {
        _tokens = null;
        revision.value++;
        await _storage.delete(key: 'buzz.enterprise.session');
      }
      rethrow; // Never retry a potentially consumed rotating refresh token automatically.
    }
  }

  Future<void> login() async {
    if (_loggingIn) throw StateError('Corporate login already in progress');
    _loggingIn = true;
    // Drain any rotating refresh before a new login can replace the same secure-storage record.
    try {
      await _refreshing;
    } catch (_) {
      /* A new browser login is the recovery path. */
    }
    final generation = ++_generation;
    StreamSubscription<Uri>? subscription;
    try {
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
      subscription = AppLinks().uriLinkStream.listen((uri) {
        if (uri.scheme != redirect.scheme ||
            uri.host != redirect.host ||
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
          'scope': 'openid profile email offline_access',
          'state': state,
          'code_challenge_method': 'S256',
          'code_challenge': challenge,
        },
      );
      if (!await launchUrl(authorize, mode: LaunchMode.externalApplication)) {
        throw StateError('Cannot open corporate login');
      }
      final uri = await callback.future.timeout(const Duration(minutes: 5));
      final tokens = await _post(_endpoint('issuer', 'oauth/token'), {
        'grant_type': 'authorization_code',
        'client_id': config['clientId'],
        'redirect_uri': redirect.toString(),
        'code': uri.queryParameters['code'],
        'code_verifier': verifier,
      });
      _validateTokens(tokens);
      final identity = _validateIdentity(
        await _post(
          _endpoint('signerUrl', 'v1/buzz/enterprise-signer/session'),
          {},
          token: tokens['access_token'] as String,
        ),
      );
      if (generation != _generation) {
        throw StateError('Corporate login changed');
      }
      await _persist(tokens, identity);
      if (generation != _generation) {
        await _storage.delete(key: 'buzz.enterprise.session');
        throw StateError('Corporate login changed');
      }
      _tokens = tokens;
      _identity = identity;
      revision.value++;
    } finally {
      await subscription?.cancel();
      _loggingIn = false;
    }
  }

  Future<void> logout() async {
    _generation++;
    try {
      await _refreshing;
    } catch (_) {
      /* Still clear credentials below. */
    }
    _tokens = null;
    _identity = null;
    revision.value++;
    await _storage.delete(key: 'buzz.enterprise.session');
  }

  Future<nostr.Event> sign({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
  }) async {
    final generation = _generation;
    final expected = pubkey;
    if (expected == null) throw StateError('Corporate login required');
    final template = {
      'kind': kind,
      'content': content,
      'tags': tags.map((t) => List<String>.of(t)).toList(),
      'created_at': createdAt ?? DateTime.now().millisecondsSinceEpoch ~/ 1000,
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
    final response = await _post(
      _endpoint('signerUrl', 'v1/buzz/enterprise-signer/events/sign'),
      {'purpose': purpose, 'event': template},
      token: await accessToken(),
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

/// Central signing seam. In enterprise builds nsec is ignored, never used as fallback.
Future<nostr.Event> signClientEvent({
  required String? nsec,
  required int kind,
  required String content,
  required List<List<String>> tags,
  int? createdAt,
}) async {
  if (enterpriseEnabled) {
    return EnterpriseIdentity.instance.sign(
      kind: kind,
      content: content,
      tags: tags,
      createdAt: createdAt,
    );
  }
  if (nsec == null || nsec.isEmpty) throw StateError('No signing identity');
  final key = nostr.Nip19.decode(payload: nsec).data;
  return nostr.Event.from(
    kind: kind,
    content: content,
    tags: tags,
    createdAt: createdAt,
    secretKey: key,
    verify: false,
  );
}
