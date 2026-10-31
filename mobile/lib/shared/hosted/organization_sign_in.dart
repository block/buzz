import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import 'browser_auth.dart';
import 'builderlab_account.dart';
import 'hosted_relay_auth.dart';

enum OrganizationSessionStatus { checking, signedOut, signingIn, signedIn }

/// Browser sign-in to the build's organization account. A completed sign-in
/// reconnects the relay so its next upgrade carries a fresh assertion. Null
/// unless this build configures an organization.
final organizationSignInProvider = Provider<OrganizationSignIn?>((ref) {
  final account = ref.watch(organizationAccountProvider);
  if (account == null) return null;
  final signIn = OrganizationSignIn(
    account: account,
    browser: BrowserAuth(origin: account.accountOrigin),
    // Only the organization's relay needs a fresh assertion.
    reconnect: () async {
      final config = ref.read(relayConfigProvider);
      if (ref.read(hostedRelayAuthProvider).organizationProtects(config)) {
        await ref.read(relaySessionProvider.notifier).reconnect();
      }
    },
  );
  ref.onDispose(signIn.dispose);
  unawaited(signIn.initialize());
  return signIn;
});

class OrganizationSignIn extends ChangeNotifier {
  OrganizationSignIn({
    required this.account,
    required this.browser,
    required this.reconnect,
  });

  final BuilderlabAccountClient account;
  final BrowserAuth browser;
  final Future<void> Function() reconnect;

  OrganizationSessionStatus status = OrganizationSessionStatus.checking;

  /// The last sign-in or session-check failure, cleared by the next attempt.
  Object? failure;

  final _subscriptions = <StreamSubscription<Object>>[];

  /// Advanced by every start or cancel, so an exchange from an abandoned
  /// attempt can never change the state of a newer one.
  int _attempt = 0;
  bool _initialized = false;
  bool _disposed = false;

  Future<void> initialize() async {
    if (_initialized || _disposed) return;
    _initialized = true;
    account.subjectChanges.addListener(_onSubject);
    _subscriptions
      ..add(browser.codes.listen((code) => unawaited(_exchange(code))))
      ..add(browser.failures.listen(_onBrowserFailure));
    await browser.initialize();
    try {
      final restored = account.account ?? await account.restore();
      _set(
        restored == null
            ? OrganizationSessionStatus.signedOut
            : OrganizationSessionStatus.signedIn,
      );
    } on AccountFailure catch (error) {
      // Unknown, for example offline. Relay connection retries the restore.
      // An unreadable record is kept so the next sign-in can replace it.
      if (error == AccountFailure.storage) failure = error;
      _set(OrganizationSessionStatus.signedOut);
    }
  }

  Future<void> signIn() =>
      // An unreadable saved session blocks every exchange, and its subject is
      // unknown. Signing in again explicitly replaces only that session.
      _begin(
        clear: failure == AccountFailure.storage && account.account == null,
      );

  /// Drops only the organization session and signs in again, for an account
  /// the relay refuses for this key. The hosted account is untouched.
  Future<void> changeAccount() => _begin(clear: true);

  Future<void> _begin({required bool clear}) async {
    if (_disposed || status == OrganizationSessionStatus.signingIn) return;
    // Busy before any await, so a second tap cannot start another attempt.
    final attempt = ++_attempt;
    failure = null;
    _set(OrganizationSessionStatus.signingIn);
    if (clear) {
      try {
        await account.clearLocalSession();
      } on AccountFailure catch (error) {
        if (attempt != _attempt) return;
        failure = error;
        _set(_currentStatus());
        return;
      }
      if (_disposed || attempt != _attempt) return;
    }
    try {
      await browser.start(const {'purpose': 'organization'});
    } on BrowserAuthFailure catch (error) {
      if (attempt != _attempt) return;
      failure = error;
      _set(_currentStatus());
    }
  }

  Future<void> cancel() async {
    ++_attempt;
    try {
      await browser.cancel();
    } on BrowserAuthFailure catch (error) {
      failure = error;
    } finally {
      _set(_currentStatus());
    }
  }

  Future<void> _exchange(BrowserAuthCode code) async {
    final attempt = _attempt;
    try {
      await account.exchange(
        code: code.code,
        verifier: code.verifier,
        // A cancelled attempt must never save its account.
        isCurrent: () => !_disposed && attempt == _attempt,
      );
    } on AccountFailure catch (error) {
      if (attempt != _attempt) return;
      failure = error;
      _set(_currentStatus());
      return;
    }
    if (_disposed || attempt != _attempt) return;
    failure = null;
    try {
      await browser.clearIntent();
    } on BrowserAuthFailure catch (error) {
      // The session is saved; a leftover intent must not keep sign-in busy.
      failure = error;
    }
    if (_disposed || attempt != _attempt) return;
    _set(OrganizationSessionStatus.signedIn);
    await reconnect();
  }

  void _onBrowserFailure(BrowserAuthFailure error) {
    if (status != OrganizationSessionStatus.signingIn) return;
    failure = error == BrowserAuthFailure.cancelled ? null : error;
    _set(_currentStatus());
  }

  void _onSubject() {
    if (status == OrganizationSessionStatus.signingIn ||
        status == OrganizationSessionStatus.checking) {
      return;
    }
    if (account.account != null) failure = null;
    _set(_currentStatus());
  }

  OrganizationSessionStatus _currentStatus() => account.account == null
      ? OrganizationSessionStatus.signedOut
      : OrganizationSessionStatus.signedIn;

  void _set(OrganizationSessionStatus next) {
    if (_disposed) return;
    status = next;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    account.subjectChanges.removeListener(_onSubject);
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    unawaited(browser.dispose());
    super.dispose();
  }
}
