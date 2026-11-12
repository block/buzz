part of 'hosted_flow.dart';

/// Account entry and discovery without requiring a hosted identity first.
extension HostedFlowAccountEntry on HostedFlow {
  /// Explicit Connect completes identity setup and discovers joined communities.
  /// Passive startup restore only checks the saved session.
  Future<void> _signInAccount({
    bool openBrowser = true,
    bool importIdentity = false,
    bool discoverOnly = false,
    bool reauthenticate = false,
  }) {
    if (_target != null ||
        stage == HostedFlowStage.working ||
        stage == HostedFlowStage.signIn) {
      return Future.value();
    }
    final generation = openBrowser ? _nextGeneration() : _generation;
    if (openBrowser) {
      _importRequested = _importRequested || importIdentity;
      reconnecting = false;
      accountOnly = true;
      _clearCommunityBatch();
      _discoverOnly = discoverOnly;
      _set(HostedFlowStage.working);
    }
    return _enqueue(() async {
      if (!openBrowser) {
        if (_target != null || stage != HostedFlowStage.idle) return;
      }
      if (openBrowser) {
        await browser.retainIntent(_accountIntent);
        if (_disposed || generation != _generation) return;
      }
      final restored = reauthenticate ? null : await account.restore();
      if (_disposed || generation != _generation) return;
      if (!openBrowser) {
        // Mounting the welcome page must not create/bind/back up a key.
        if (_importRequested && restored != null) {
          final saved = await store.forSubject(restored.subject).read();
          if (_disposed || generation != _generation) return;
          if (saved != null) {
            // Also repair intents saved before import completion was recorded.
            _importRequested = false;
            await browser.retainIntent(_accountIntent);
            if (_disposed || generation != _generation) return;
          }
        }
        if (_importRequested && restored != null) {
          canCreateIdentity = false;
          requiredIdentityPubkey = null;
          _set(HostedFlowStage.setupIdentity);
        } else {
          _set(HostedFlowStage.idle);
        }
      } else if (restored != null) {
        await _continue(generation);
      } else {
        _set(HostedFlowStage.signIn);
        await browser.start(_accountIntent);
      }
    });
  }
}
