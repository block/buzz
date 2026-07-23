import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:sentry_flutter/sentry_flutter.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'sentry_config.dart';

const diagnosticsConsentPreferenceKey = 'buzz_crash_reporting_consent';
const _diagnosticsEnabledByDefault = true;

typedef DiagnosticsLog = void Function(String message);

abstract interface class CrashReporter {
  Future<void> initialize(SentryConfig config);
  Future<void> close();
}

class SentryCrashReporter implements CrashReporter {
  const SentryCrashReporter();

  @override
  Future<void> initialize(SentryConfig config) {
    return SentryFlutter.init(config.applyTo);
  }

  @override
  Future<void> close() => Sentry.close();
}

class DiagnosticsController extends ChangeNotifier {
  DiagnosticsController({
    required SharedPreferences preferences,
    required SentryConfig config,
    required CrashReporter crashReporter,
    DiagnosticsLog? log,
  }) : _preferences = preferences,
       _config = config,
       _crashReporter = crashReporter,
       _log = log ?? debugPrint,
       _consentGranted =
           preferences.getBool(diagnosticsConsentPreferenceKey) ??
           _diagnosticsEnabledByDefault;

  final SharedPreferences _preferences;
  final SentryConfig _config;
  final CrashReporter _crashReporter;
  final DiagnosticsLog _log;

  bool _consentGranted;
  bool _initialized = false;
  Future<void> _pendingOperation = Future.value();

  bool get consentGranted => _consentGranted;
  bool get isConfigured => _config.isConfigured;

  /// Applies persisted consent before the app starts rendering.
  Future<void> applyStartupConsent() => _serialize(_applyCurrentConsent);

  /// Persists consent and applies it immediately.
  Future<void> setConsent(bool granted) {
    return _serialize(() async {
      if (_consentGranted == granted) {
        // A failed close keeps the runtime initialized even though the
        // persisted preference and visible control are already off. Allow the
        // same revocation to retry teardown until it succeeds.
        if (!granted && _initialized) {
          await _applyCurrentConsent();
        }
        return;
      }

      if (granted && !_config.isConfigured) {
        _log(
          'Diagnostics unchanged: SENTRY_DSN is empty; consent not enabled.',
        );
        throw StateError('Crash reporting is unavailable in this build');
      }

      final previousConsent = _consentGranted;
      var consentPersisted = false;
      _consentGranted = granted;
      notifyListeners();
      try {
        final persisted = await _preferences.setBool(
          diagnosticsConsentPreferenceKey,
          granted,
        );
        if (!persisted) {
          throw StateError('Failed to persist diagnostics consent');
        }
        consentPersisted = true;
        await _applyCurrentConsent();
      } on Object {
        // A persisted revocation must survive teardown failure. Otherwise the
        // next launch could initialize crash reporting against the user's
        // explicit choice.
        if (!granted && consentPersisted) {
          rethrow;
        }
        _consentGranted = previousConsent;
        final rolledBack = await _preferences.setBool(
          diagnosticsConsentPreferenceKey,
          previousConsent,
        );
        notifyListeners();
        if (!rolledBack) {
          throw StateError('Failed to roll back diagnostics consent');
        }
        rethrow;
      }
    });
  }

  Future<void> _applyCurrentConsent() async {
    if (!_consentGranted) {
      if (_initialized) {
        await _crashReporter.close();
        _initialized = false;
        _log('Diagnostics disabled: user consent is off; Sentry closed.');
      } else {
        _log(
          'Diagnostics disabled: user consent is off; Sentry not initialized.',
        );
      }
      return;
    }

    if (!_config.isConfigured) {
      _log(
        'Diagnostics disabled: SENTRY_DSN is empty; Sentry not initialized.',
      );
      return;
    }

    if (_initialized) {
      _log('Diagnostics already enabled: Sentry initialization skipped.');
      return;
    }

    await _crashReporter.initialize(_config);
    _initialized = true;
    _log('Diagnostics enabled: Sentry initialized after user consent.');
  }

  Future<void> _serialize(Future<void> Function() operation) {
    final result = _pendingOperation.then((_) => operation());
    _pendingOperation = result.catchError((Object _) {});
    return result;
  }
}
