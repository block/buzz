import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/push/push_bootstrap.dart';
import '../../shared/push/push_bridge.dart';
import 'age_signal_provider.dart';

/// Delay between failed age-gate snapshot transitions.
const ageSignalPushSnapshotInitialRetryDelay = Duration(seconds: 5);

/// Maximum delay between failed age-gate snapshot transitions.
const ageSignalPushSnapshotMaximumRetryDelay = Duration(minutes: 5);

/// Exponential retry delay for a zero-based consecutive failure count.
Duration ageSignalPushSnapshotRetryDelay(int failures) {
  final boundedFailures = failures.clamp(0, 6);
  final seconds =
      ageSignalPushSnapshotInitialRetryDelay.inSeconds * (1 << boundedFailures);
  return Duration(
    seconds: seconds.clamp(0, ageSignalPushSnapshotMaximumRetryDelay.inSeconds),
  );
}

/// Waits before retrying a failed age-gate snapshot transition.
typedef AgeSignalPushSnapshotRetryWait =
    Future<void> Function(Duration duration);

/// Retry wait used by the launch age gate's notification snapshot boundary.
final ageSignalPushSnapshotRetryWaitProvider =
    Provider<AgeSignalPushSnapshotRetryWait>((ref) {
      return Future<void>.delayed;
    });

/// Native notification purge performed once restriction is confirmed.
final ageRestrictedNotificationPurgerProvider =
    Provider<Future<void> Function()>(
      (ref) => purgeAgeRestrictedBuzzNotifications,
    );

/// Delay between successful maintenance purges while restriction remains active.
const ageRestrictedNotificationMaintenanceDelay = Duration(seconds: 30);

/// Number of delayed purge passes after an initial successful purge.
const ageRestrictedNotificationMaintenancePurgeLimit = 3;

/// Schedules a recheck for interactions donated by stale extensions.
final ageRestrictedNotificationMaintenanceScheduleProvider =
    Provider<VoidCallback Function(VoidCallback)>((ref) {
      if (defaultTargetPlatform != TargetPlatform.iOS) {
        return (_) => () {};
      }
      return (callback) {
        final timer = Timer(
          ageRestrictedNotificationMaintenanceDelay,
          callback,
        );
        return timer.cancel;
      };
    });

/// Starts the push lifecycle only after the launch age check allows access.
class AgeSignalPushBootstrap extends HookConsumerWidget {
  /// Creates the production push boundary around [child].
  const AgeSignalPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(ageSignalProvider);
    final suspendSnapshot = ref.watch(
      suspendCommunitySnapshotForAgeCheckProvider,
    );
    final resumeSnapshot = ref.watch(
      resumeCommunitySnapshotAfterAgeCheckProvider,
    );
    final waitBeforeRetry = ref.watch(ageSignalPushSnapshotRetryWaitProvider);
    final retryGeneration = useState(0);
    final consecutiveFailures = useRef(0);
    final previousState = useRef<AgeSignalState?>(null);

    useEffect(
      () {
        if (previousState.value != state) {
          previousState.value = state;
          consecutiveFailures.value = 0;
        }
        var cancelled = false;
        unawaited(() async {
          try {
            await (state == AgeSignalState.allowed
                ? resumeSnapshot()
                : suspendSnapshot());
            consecutiveFailures.value = 0;
          } catch (_) {
            final delay = ageSignalPushSnapshotRetryDelay(
              consecutiveFailures.value,
            );
            await waitBeforeRetry(delay);
            if (!cancelled) {
              consecutiveFailures.value += 1;
              retryGeneration.value += 1;
            }
          }
        }());
        return () => cancelled = true;
      },
      [
        state,
        suspendSnapshot,
        resumeSnapshot,
        waitBeforeRetry,
        retryGeneration.value,
      ],
    );

    return switch (state) {
      AgeSignalState.allowed => BuzzPushBootstrap(child: child),
      AgeSignalState.restricted => _AgeRestrictedPushCleanup(child: child),
      AgeSignalState.checking || AgeSignalState.retryableFailure => child,
    };
  }
}

class _AgeRestrictedPushCleanup extends HookConsumerWidget {
  const _AgeRestrictedPushCleanup({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communitiesReady = ref.watch(communityListProvider).hasValue;
    final waitBeforeRetry = ref.watch(ageSignalPushSnapshotRetryWaitProvider);
    final purgeNotifications = ref.watch(
      ageRestrictedNotificationPurgerProvider,
    );
    final scheduleMaintenancePurge = ref.watch(
      ageRestrictedNotificationMaintenanceScheduleProvider,
    );
    final resumeGeneration = useState(0);
    final retryGeneration = useState(0);
    final consecutiveFailures = useRef(0);
    final purgeRetryGeneration = useState(0);
    final consecutivePurgeFailures = useRef(0);
    final remainingMaintenancePurges = useRef(
      ageRestrictedNotificationMaintenancePurgeLimit,
    );

    useEffect(() {
      final listener = AppLifecycleListener(
        onResume: () {
          if (ref.read(communityListProvider).hasError) {
            ref.invalidate(communityListProvider);
          }
          remainingMaintenancePurges.value =
              ageRestrictedNotificationMaintenancePurgeLimit;
          resumeGeneration.value += 1;
        },
      );
      return listener.dispose;
    }, const []);

    useEffect(
      () {
        var cancelled = false;
        VoidCallback? cancelMaintenancePurge;
        unawaited(() async {
          try {
            await purgeNotifications();
            consecutivePurgeFailures.value = 0;
            if (!cancelled && remainingMaintenancePurges.value > 0) {
              remainingMaintenancePurges.value -= 1;
              cancelMaintenancePurge = scheduleMaintenancePurge(() {
                if (!cancelled) {
                  purgeRetryGeneration.value += 1;
                }
              });
            }
          } catch (error, stackTrace) {
            reportPushLeaseCleanupError(error, stackTrace);
            final delay = ageSignalPushSnapshotRetryDelay(
              consecutivePurgeFailures.value,
            );
            await waitBeforeRetry(delay);
            if (!cancelled) {
              consecutivePurgeFailures.value += 1;
              purgeRetryGeneration.value += 1;
            }
          }
        }());
        return () {
          cancelled = true;
          cancelMaintenancePurge?.call();
        };
      },
      [
        purgeNotifications,
        scheduleMaintenancePurge,
        waitBeforeRetry,
        resumeGeneration.value,
        purgeRetryGeneration.value,
      ],
    );

    useEffect(
      () {
        var cancelled = false;
        if (communitiesReady) {
          unawaited(() async {
            try {
              await ref
                  .read(communityListProvider.notifier)
                  .enforceAgeRestrictionOnPush();
              consecutiveFailures.value = 0;
            } catch (error, stackTrace) {
              reportPushLeaseCleanupError(error, stackTrace);
              final delay = ageSignalPushSnapshotRetryDelay(
                consecutiveFailures.value,
              );
              await waitBeforeRetry(delay);
              if (!cancelled) {
                consecutiveFailures.value += 1;
                retryGeneration.value += 1;
              }
            }
          }());
        }
        return () => cancelled = true;
      },
      [
        communitiesReady,
        waitBeforeRetry,
        resumeGeneration.value,
        retryGeneration.value,
      ],
    );

    return child;
  }
}
