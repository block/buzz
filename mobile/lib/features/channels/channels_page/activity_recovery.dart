part of '../channels_page.dart';

/// Observes route visibility without rebuilding the channel inventory.
class _ActivityRecovery extends HookConsumerWidget {
  const _ActivityRecovery({required this.activity, required this.child});
  final Map<String, AsyncValue<int?>> activity;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activityScope = (
      ref.watch(relayConfigProvider.select((config) => config.baseUrl)),
      ref.watch(myPubkeyProvider),
    );
    final notice = useMemoized(() => ObjectRef<OverlayEntry?>(null), [
      activityScope,
    ]);
    final failures = useMemoized(() => <String>{}, [activityScope]);
    final automaticAttempts = useMemoized(() => ObjectRef(0), [activityScope]);
    failures.retainAll(activity.keys);
    for (final MapEntry(key: id, value: status) in activity.entries) {
      if (status.hasError) {
        failures.add(id);
      } else if (!status.isLoading) {
        failures.remove(id);
      }
    }
    final unavailable = failures.isNotEmpty;
    if (!unavailable) automaticAttempts.value = 0;

    bool retryFailures() {
      var started = false;
      for (final id in failures.toList()) {
        final provider = channelFilterActivityProvider(id);
        if (ref.read(provider).isLoading) continue;
        ref.invalidate(provider);
        started = true;
      }
      return started;
    }

    // Compact detail routes cover the list within Home's route and mute its
    // tickers. On wide layouts the Messages list stays visible beside detail.
    final listOnstage = TickerMode.valuesOf(context).enabled;
    final routeCurrent = ModalRoute.isCurrentOf(context) ?? true;
    final messagesVisible = listOnstage && routeCurrent;
    final canRetry =
        messagesVisible &&
        ref.watch(appLifecycleProvider) == AppLifecycleState.resumed &&
        ref.watch(relaySessionProvider.select((s) => s.status)) ==
            SessionStatus.connected;
    final hasRetryableFailure = failures.any((id) => !activity[id]!.isLoading);
    final attempts = automaticAttempts.value;
    useEffect(() {
      if (!hasRetryableFailure || !canRetry || attempts >= 3) return null;
      // Back off over consecutive failed rounds and stagger relay traffic
      // across clients. Wait for a failed lookup before scheduling a round.
      final delayMs = 10_000 * (1 << attempts);
      final spread = delayMs ~/ 5;
      final timer = Timer(
        Duration(milliseconds: delayMs - spread + Random().nextInt(2 * spread)),
        () {
          if (retryFailures()) automaticAttempts.value++;
        },
      );
      return timer.cancel;
    }, [hasRetryableFailure, canRetry, automaticAttempts, attempts]);

    void removeNotice() {
      notice.value?.remove();
      notice.value?.dispose();
      notice.value = null;
    }

    useEffect(() => removeNotice, [notice]);
    useEffect(() {
      if (!messagesVisible) {
        removeNotice();
        return null;
      }
      var active = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!active || !context.mounted) return;
        if (notice.value != null) {
          notice.value!.markNeedsBuild();
        } else if (unavailable) {
          late final OverlayEntry entry;
          entry = OverlayEntry(
            builder: (_) => IosCompositionLayer(
              child: Stack(
                children: [
                  ReconnectCapsule(
                    visible: failures.isNotEmpty,
                    message: 'Some activity couldn’t load',
                    actionLabel: 'Retry',
                    onHidden: () {
                      WidgetsBinding.instance.addPostFrameCallback((_) {
                        if (identical(notice.value, entry) &&
                            failures.isEmpty) {
                          removeNotice();
                        }
                      });
                    },
                    onSignIn: () {
                      if (!context.mounted) return;
                      retryFailures();
                    },
                  ),
                ],
              ),
            ),
          );
          notice.value = entry;
          Overlay.of(context, rootOverlay: true).insert(entry);
        }
      });
      return () => active = false;
    }, [notice, unavailable, messagesVisible]);
    return child;
  }
}
