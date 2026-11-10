import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Refreshes account destinations only while the entry screen is active.
/// A failed refresh preserves the last successful result for this account.
bool useKnownCommunities({
  required String? accountKey,
  required bool active,
  required Future<bool> Function() load,
}) {
  final available = useMemoized(() => ValueNotifier(false), [accountKey]);
  useEffect(() => available.dispose, [available]);
  useListenable(available);
  final latestLoad = useRef(load)..value = load;
  useEffect(() {
    if (accountKey == null || !active) return null;
    var disposed = false;
    var loading = false;
    Future<void> refresh() async {
      if (loading) return;
      loading = true;
      try {
        final result = await latestLoad.value();
        if (!disposed) available.value = result;
      } catch (_) {
        // Discovery is optional; an unavailable service is not an empty list.
      } finally {
        loading = false;
      }
    }

    Timer? timer;
    void lifecycleChanged(AppLifecycleState? state) {
      timer?.cancel();
      timer = null;
      if (state != null && state != AppLifecycleState.resumed) return;
      unawaited(refresh());
      timer = Timer.periodic(const Duration(seconds: 30), (_) {
        unawaited(refresh());
      });
    }

    // Lifecycle callbacks must stop timers immediately: paused apps do not
    // render a frame, so a rebuild-driven effect may never run its cleanup.
    final listener = AppLifecycleListener(onStateChange: lifecycleChanged);
    lifecycleChanged(WidgetsBinding.instance.lifecycleState);
    return () {
      disposed = true;
      timer?.cancel();
      listener.dispose();
    };
  }, [accountKey, active, available]);
  return available.value;
}
