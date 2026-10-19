import '../storage/serial_queue.dart';
import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:app_links/app_links.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'package:url_launcher/url_launcher.dart';

import '../storage/secure_storage.dart';

import 'hosted_environment.dart';

/// Safe to show as a generic retry prompt. Never carries a URL or server text.
enum BrowserAuthFailure { restartLogin, cancelled, unavailable, storage }

class BrowserAuthCode {
  const BrowserAuthCode(this.code, this.verifier, this.intent);
  final String code;
  final String verifier;
  final Map<String, String> intent;
}

/// One app-scoped owner. Subscribe to [codes] and [failures] before [initialize].
/// Callbacks bypass the community deep-link FIFO. A consumed code is never
/// replayed automatically: exchange failure requires a fresh browser login.
class BrowserAuth {
  BrowserAuth({
    FlutterSecureStorage? storage,
    Future<String?> Function(Uri)? openBrowser,
    DateTime Function()? now,
  }) : _storage = storage ?? accountSecureStorage,
       _openBrowser = openBrowser ?? _openPlatformBrowser,
       _now = now ?? DateTime.now;

  static const callback = 'xyz.block.buzz.mobile:/auth/callback';
  static final _attemptKey =
      'buzz_browser_auth_attempt_v1_${HostedEnvironment.origin.host}';
  static final _intentKey =
      'buzz_browser_auth_intent_v1_${HostedEnvironment.origin.host}';
  static const _channel = MethodChannel('buzz/browser_auth');
  final FlutterSecureStorage _storage;
  final Future<String?> Function(Uri) _openBrowser;
  final DateTime Function() _now;
  final _codes = StreamController<BrowserAuthCode>.broadcast();
  final _failures = StreamController<BrowserAuthFailure>.broadcast();
  Stream<BrowserAuthCode> get codes => _codes.stream;
  Stream<BrowserAuthFailure> get failures => _failures.stream;
  StreamSubscription<Uri>? _links;
  final _queue = SerialQueue();
  bool _disposed = false;
  bool _initialized = false;
  int _generation = 0;
  String? _activeState;
  String? _consumedCallbackHash;

  Future<T> _serial<T>(Future<T> Function() task) {
    final result = _queue.run(() {
      if (_disposed) throw BrowserAuthFailure.cancelled;
      return task();
    });
    return result;
  }

  Future<void> initialize() async {
    if (_initialized || _disposed) return;
    _initialized = true;
    final links = AppLinks();
    _links = links.uriLinkStream.listen((uri) {
      if (uri.scheme == 'xyz.block.buzz.mobile') {
        unawaited(receive(uri).catchError((Object _) {}));
      }
    }, onError: (Object _) => _fail(BrowserAuthFailure.unavailable));
    try {
      final initial = await links.getInitialLink();
      if (initial != null && initial.scheme == 'xyz.block.buzz.mobile') {
        await receive(initial);
      }
    } catch (_) {
      _fail(BrowserAuthFailure.unavailable);
    }
  }

  /// Intent is opaque to authentication; callers validate its relay/challenge
  /// before acting. Retained separately so missing state never loses navigation.
  Future<Map<String, String>?> retainedIntent() => _serial(() async {
    try {
      final raw = await _storage.read(key: _intentKey);
      if (raw == null) return null;
      return Map<String, String>.from(jsonDecode(raw) as Map);
    } catch (_) {
      throw BrowserAuthFailure.storage;
    }
  });

  /// Persist setup intent before remote work or acknowledging key transfer.
  Future<void> retainIntent(Map<String, String> intent) => _serial(() async {
    final raw = jsonEncode(intent);
    if (raw.length > 16384) throw BrowserAuthFailure.storage;
    await _storage.write(key: _intentKey, value: raw);
    if (await _storage.read(key: _intentKey) != raw) {
      throw BrowserAuthFailure.storage;
    }
  });

  Future<void> start(Map<String, String> intent) async {
    final generation = ++_generation;
    final uri = await _serial(() async {
      if (jsonEncode(intent).length > 16384) throw BrowserAuthFailure.storage;
      final random = Random.secure();
      String secret() => base64UrlEncode(
        List.generate(32, (_) => random.nextInt(256)),
      ).replaceAll('=', '');
      final verifier = secret();
      final state = secret();
      _activeState = state;
      final record = jsonEncode({
        'verifier': verifier,
        'state': state,
        'created': _now().toUtc().millisecondsSinceEpoch,
        'intent': intent,
      });
      try {
        await _storage.write(key: _intentKey, value: jsonEncode(intent));
        await _storage.write(key: _attemptKey, value: record);
        if (await _storage.read(key: _attemptKey) != record) {
          throw BrowserAuthFailure.storage;
        }
      } catch (_) {
        throw BrowserAuthFailure.storage;
      }
      return HostedEnvironment.origin.replace(
        path: '/api/goose/v1/auth/login',
        queryParameters: {
          'type': 'cli',
          'product': 'buzz',
          'returnTo': callback,
          'state': state,
          'code_challenge': base64UrlEncode(
            sha256.convert(ascii.encode(verifier)).bytes,
          ).replaceAll('=', ''),
          'code_challenge_method': 'S256',
        },
      );
    });
    if (_disposed || generation != _generation) return;
    try {
      final result = await _openBrowser(uri);
      if (!_disposed && generation == _generation && result != null) {
        await receive(Uri.parse(result));
      }
    } on PlatformException catch (error) {
      if (generation == _generation) {
        _fail(
          error.code == 'cancelled'
              ? BrowserAuthFailure.cancelled
              : BrowserAuthFailure.unavailable,
        );
      }
    } catch (_) {
      if (generation == _generation) _fail(BrowserAuthFailure.unavailable);
    }
  }

  Future<void> receive(Uri uri) {
    final generation = _generation;
    return _serial(() async {
      if (generation != _generation) return;
      void fail(BrowserAuthFailure failure) {
        if (generation == _generation) _fail(failure);
      }

      // Reject noncanonical destination and duplicate query fields before reading
      // secrets. Neither invalid state nor malformed callbacks mutate the session.
      final callbackHash = sha256
          .convert(utf8.encode(uri.toString()))
          .toString();
      if (callbackHash == _consumedCallbackHash) return;
      final query = uri.queryParametersAll;
      if (uri.toString().split('?').first != callback ||
          uri.hasFragment ||
          query.values.any((v) => v.length != 1) ||
          query.keys.any((k) => k != 'state' && k != 'code' && k != 'error')) {
        if (_activeState == null) fail(BrowserAuthFailure.restartLogin);
        return;
      }
      try {
        final saved = await _storage.read(key: _attemptKey);
        if (saved == null) {
          fail(BrowserAuthFailure.restartLogin);
          return;
        }
        final record = jsonDecode(saved) as Map<String, dynamic>;
        final expectedState = record['state'];
        if (expectedState is! String ||
            !RegExp(r'^[A-Za-z0-9_-]{43}$').hasMatch(expectedState)) {
          throw BrowserAuthFailure.storage;
        }
        final age =
            _now().toUtc().millisecondsSinceEpoch - (record['created'] as int);
        if (age < 0 || age > const Duration(minutes: 10).inMilliseconds) {
          fail(BrowserAuthFailure.restartLogin);
          return;
        }
        if (query['state']?.single != expectedState) {
          if (_activeState != expectedState) {
            fail(BrowserAuthFailure.restartLogin);
          }
          return;
        }
        if (generation != _generation) return;
        final code = query['code']?.single;
        if (query.containsKey('error') ||
            code == null ||
            code.isEmpty ||
            code.length > 4096) {
          fail(BrowserAuthFailure.restartLogin);
          return;
        }
        final verifier = record['verifier'] as String;
        if (!RegExp(r'^[A-Za-z0-9_-]{43}$').hasMatch(verifier)) {
          throw BrowserAuthFailure.storage;
        }
        final intent = Map<String, String>.from(record['intent'] as Map);
        await _storage.delete(key: _attemptKey);
        if (await _storage.read(key: _attemptKey) != null) {
          throw BrowserAuthFailure.storage;
        }
        _activeState = null;
        _consumedCallbackHash = callbackHash;
        if (!_disposed && generation == _generation) {
          _codes.add(BrowserAuthCode(code, verifier, Map.unmodifiable(intent)));
        }
      } catch (_) {
        fail(BrowserAuthFailure.storage);
      }
    });
  }

  Future<void> cancel() async {
    ++_generation;
    _activeState = null;
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      try {
        await _channel.invokeMethod<void>('cancel');
      } catch (_) {
        /* No session may be active. */
      }
    }
    await _serial(() => _storage.delete(key: _attemptKey));
  }

  /// Call only after completing or explicitly abandoning the retained intent.
  Future<void> clearIntent() => _serial(() => _storage.delete(key: _intentKey));

  void _fail(BrowserAuthFailure failure) {
    if (!_disposed) _failures.add(failure);
  }

  Future<void> dispose() async {
    _disposed = true;
    ++_generation;
    await _links?.cancel();
    await _codes.close();
    await _failures.close();
  }

  static Future<String?> _openPlatformBrowser(Uri uri) async {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return _channel.invokeMethod<String>('start', {
        'url': uri.toString(),
        if (kDebugMode &&
            const bool.fromEnvironment('BUZZ_DEBUG_PRIVATE_LOGIN'))
          'ephemeral': 'true',
      });
    }
    if (defaultTargetPlatform == TargetPlatform.android &&
        await launchUrl(uri, mode: LaunchMode.inAppBrowserView)) {
      return null;
    }
    throw BrowserAuthFailure.unavailable;
  }
}
