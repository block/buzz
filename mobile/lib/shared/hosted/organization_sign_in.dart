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
    } on AccountFailure {
      // Unknown, for example offline. Relay connection retries the restore.
      _set(OrganizationSessionStatus.signedOut);
    }
  }

  Future<void> signIn() async {
    if (_disposed || status == OrganizationSessionStatus.signingIn) return;
    failure = null;
    _set(OrganizationSessionStatus.signingIn);
    try {
      await browser.start(const {'purpose': 'organization'});
    } on BrowserAuthFailure catch (error) {
      failure = error;
      _set(_currentStatus());
    }
  }

  Future<void> cancel() async {
    await browser.cancel();
    _set(_currentStatus());
  }

  Future<void> _exchange(BrowserAuthCode code) async {
    try {
      await account.exchange(code: code.code, verifier: code.verifier);
      await browser.clearIntent();
      if (_disposed) return;
      failure = null;
      _set(OrganizationSessionStatus.signedIn);
      await reconnect();
    } on AccountFailure catch (error) {
      failure = error;
      _set(_currentStatus());
    } on BrowserAuthFailure catch (error) {
      failure = error;
      _set(_currentStatus());
    }
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
