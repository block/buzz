import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart' show defaultTargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community.dart';
import '../community/community_provider.dart';
import '../diagnostics/diagnostic_reporter.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import '../relay/signed_event_relay.dart';
import 'android_push_keystore.dart';
import 'dev_push_lease.dart';
import 'push_bridge.dart';
import 'push_lease_revocation_outbox.dart';
import 'push_relay_capability_provider.dart';
import 'push_subscription.dart';
import 'push_subscription_provider.dart';
import 'push_setup_recovery.dart';

const _pushBootstrapRetryDelay = Duration(seconds: 5);
const _pushBootstrapMaxRetryDelay = Duration(minutes: 15);

@visibleForTesting
class BuzzPushAttemptGate {
  BuzzPushAttemptGate({
    this.retryDelay = _pushBootstrapRetryDelay,
    this.maxRetryDelay = _pushBootstrapMaxRetryDelay,
    double Function()? jitter,
  }) : _jitter = jitter ?? Random().nextDouble;

  /// Delay before the first retry of a failing attempt. Each further
  /// consecutive failure of the same attempt doubles it, up to
  /// [maxRetryDelay], so a persistent refusal (for example a gateway that
  /// blocks this device) is not retried every few seconds forever.
  final Duration retryDelay;
  final Duration maxRetryDelay;
  final double Function() _jitter;
  String? _attempt;
  Timer? _retryTimer;
  String? _failingAttempt;
  int _consecutiveFailures = 0;

  bool tryBegin(String attempt) {
    if (_attempt == attempt) return false;
    _retryTimer?.cancel();
    _retryTimer = null;
    _attempt = attempt;
    return true;
  }

  void failed(String attempt, {required VoidCallback retry}) {
    if (_attempt != attempt) return;
    if (_failingAttempt != attempt) {
      _failingAttempt = attempt;
      _consecutiveFailures = 0;
    }
    _schedule(attempt, _backoff(_consecutiveFailures++), retry);
  }

  /// Between half and all of the capped exponential delay, so devices that
  /// failed together do not retry together.
  Duration _backoff(int priorFailures) {
    final ceiling = min(
      maxRetryDelay.inMilliseconds,
      retryDelay.inMilliseconds * (1 << min(priorFailures, 20)),
    );
    final half = ceiling ~/ 2;
    return Duration(
      milliseconds: half + (_jitter() * (ceiling - half)).floor(),
    );
  }

  void _schedule(String attempt, Duration delay, VoidCallback retry) {
    _retryTimer?.cancel();
    _retryTimer = Timer(delay, () {
      _retryTimer = null;
      if (_attempt != attempt) return;
      _attempt = null;
      retry();
    });
  }

  /// Schedules a retry after a success (for example a renewal) or after a
  /// failure that picks its own delay. Ends any failure streak.
  void retryAfter(
    String attempt, {
    required Duration delay,
    required VoidCallback retry,
  }) {
    if (_attempt != attempt) return;
    _endFailureStreak();
    _schedule(attempt, delay, retry);
  }

  void complete(String attempt) {
    if (_attempt != attempt) return;
    _retryTimer?.cancel();
    _retryTimer = null;
    _attempt = null;
    _endFailureStreak();
  }

  /// Records that [attempt] succeeded without making it runnable again, so a
  /// later failure starts from the first retry delay.
  void succeeded(String attempt) {
    if (_attempt == attempt) _endFailureStreak();
  }

  void _endFailureStreak() {
    _failingAttempt = null;
    _consecutiveFailures = 0;
  }

  void dispose() => _retryTimer?.cancel();
}

@visibleForTesting
String buzzPushPublicationAttemptKey({
  required String communityId,
  required String relayBaseUrl,
  required String token,
  required BuzzPushLeaseDescriptor descriptor,
  required List<BuzzPushSubscription> subscriptions,
}) => [
  communityId,
  relayBaseUrl,
  token,
  descriptor.executorKeyId,
  descriptor.executorPubkey,
  descriptor.hGrammar,
  buzzPushSubscriptionsFingerprint(subscriptions),
].join('|');

/// Identifies a pass over pending opt-out tombstones by the communities it
/// covers, so a community whose tombstone appears while another is backing
/// off runs now instead of waiting out that delay. Not by generation: each
/// pass advances it, and that must not restart the backoff.
@visibleForTesting
String buzzPushTombstoneAttemptKey(Iterable<Community> pending) =>
    [for (final community in pending) community.id].join('|');

@visibleForTesting
bool buzzPushLifecycleEnabled({
  required Community? community,
  required BuzzPushLeaseDescriptor? descriptor,
}) =>
    Env.pushGatewayConfigured &&
    community?.pushNotificationsEnabled == true &&
    descriptor != null;

/// Starts APNs registration when the active community opts in.
@visibleForTesting
class BuzzPushRegistrationBootstrap extends HookWidget {
  const BuzzPushRegistrationBootstrap({
    required this.shouldRegister,
    required this.attemptKey,
    required this.child,
    this.startRegistration = startBuzzPushRegistration,
    super.key,
  });

  final bool shouldRegister;
  final String attemptKey;
  final Widget child;
  final Future<void> Function() startRegistration;

  @override
  Widget build(BuildContext context) {
    final attemptGate = useMemoized(BuzzPushAttemptGate.new);
    final retry = useState(0);
    useEffect(() => attemptGate.dispose, const []);
    useEffect(() {
      if (!shouldRegister || !attemptGate.tryBegin(attemptKey)) return null;
      unawaited(() async {
        try {
          await startRegistration();
          attemptGate.succeeded(attemptKey);
        } catch (error, stack) {
          attemptGate.failed(
            attemptKey,
            retry: () {
              if (context.mounted) retry.value += 1;
            },
          );
          debugPrint('Push registration bootstrap failed: $error');
          debugPrintStack(stackTrace: stack);
        }
      }());
      return null;
    }, [shouldRegister, attemptKey, retry.value]);
    return child;
  }
}

@visibleForTesting
Future<int> publishBuzzPushLeaseRecoverably({
  required Future<int> Function() reserveGeneration,
  required Future<void> Function(int generation) publish,
  required Future<bool> Function(int generation) markAccepted,
}) async {
  final generation = await reserveGeneration();
  await publish(generation);
  if (!await markAccepted(generation)) {
    throw StateError('A newer push lease superseded the published generation.');
  }
  return generation;
}

typedef _PushInputs = ({
  BuzzPushLeaseRevocationOutbox revocationOutbox,
  SessionState session,
  List<Community> communities,
  bool communitiesLoaded,
  RelayConfig config,
  Community? community,
  String? memberPubkey,
  BuzzPushLeaseDescriptor? descriptor,
});

// Convert synchronous provider failures into data before they reach a widget
// build. Dependencies remain watched so corrected inputs recover immediately.
final _pushInputsProvider = Provider.autoDispose<AsyncValue<_PushInputs>>((
  ref,
) {
  try {
    return AsyncData((
      revocationOutbox: ref.watch(buzzPushLeaseRevocationOutboxProvider),
      session: ref.watch(relaySessionProvider),
      communities: ref.watch(communityListProvider).value ?? const [],
      communitiesLoaded: ref.watch(communityListProvider).hasValue,
      config: ref.watch(relayConfigProvider),
      community: ref.watch(activeCommunityProvider).value,
      memberPubkey: ref.watch(myPubkeyProvider),
      descriptor: ref.watch(currentRelayPushDescriptorProvider).value,
    ));
  } catch (error, stack) {
    reportPushSetupError('initialization', error, stack);
    return AsyncError(error, stack);
  }
});

// Pull cold-start notification routing after runApp. A failed platform call
// must not prevent the first frame or disappear as an unobserved future.
final _pendingNotificationResponseProvider = FutureProvider.autoDispose<void>((
  ref,
) async {
  try {
    await syncPendingBuzzPushNotificationResponse();
  } catch (error, stack) {
    reportPushSetupError('pending notification response', error, stack);
    rethrow;
  }
}, retry: pushSetupRetry);

/// Observable startup/synchronization status, independent of core app rendering.
final buzzPushSetupStatusProvider = Provider.autoDispose<AsyncValue<void>>((
  ref,
) {
  final inputs = ref.watch(_pushInputsProvider);
  final response = ref.watch(_pendingNotificationResponseProvider);
  final subscriptions = ref.watch(pushSubscriptionSyncProvider);
  if (response.hasError) return response;
  if (inputs.hasError) return AsyncError(inputs.error!, inputs.stackTrace!);
  return subscriptions;
});

/// Retries push-owned setup dependencies without resetting the app or session.
void retryBuzzPushSetup(WidgetRef ref) {
  // Unrelated setup failures must not dispose a healthy cleanup worker.
  // Reconstruct only a worker whose provider actually failed to initialize.
  try {
    ref.read(buzzPushLeaseRevocationOutboxProvider);
  } catch (error, stack) {
    reportPushSetupError('revocation worker retry', error, stack);
    ref.invalidate(buzzPushLeaseRevocationOutboxProvider);
  }
  ref.invalidate(currentRelayPushDescriptorProvider);
  ref.invalidate(pushSubscriptionSyncProvider);
  ref.invalidate(_pendingNotificationResponseProvider);
  ref.invalidate(_pushInputsProvider);
}

/// Starts push independently of [child], which stays mounted during failures.
class BuzzPushBootstrap extends HookConsumerWidget {
  const BuzzPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final inputs = ref.watch(_pushInputsProvider);
    ref.watch(buzzPushSetupStatusProvider);
    final failures = useRef(0);
    useEffect(() {
      if (!inputs.hasError) {
        failures.value = 0;
        return null;
      }
      final delay = pushSetupRetry(failures.value++, inputs.error!);
      if (delay == null) return null;
      final timer = Timer(delay, () {
        if (context.mounted) retryBuzzPushSetup(ref);
      });
      return timer.cancel;
    }, [inputs]);
    // A stable sibling slot preserves navigation and widget state when push
    // fails, retries, or becomes available after startup.
    return Stack(
      textDirection: TextDirection.ltr,
      children: [
        child,
        if (inputs.asData case final data?)
          _BuzzPushLifecycle(inputs: data.value),
      ],
    );
  }
}

class _BuzzPushLifecycle extends HookConsumerWidget {
  const _BuzzPushLifecycle({required this.inputs});

  final _PushInputs inputs;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(apnsDeviceToken);
    useListenable(fcmDeviceToken);
    final publicationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final tombstoneAttempt = useMemoized(BuzzPushAttemptGate.new);
    final reconcileAttempt = useMemoized(BuzzPushAttemptGate.new);
    final publicationRetry = useState(0);
    final tombstoneRetry = useState(0);
    final reconcileRetry = useState(0);
    final (
      :revocationOutbox,
      :session,
      :communities,
      :communitiesLoaded,
      :config,
      :community,
      :memberPubkey,
      :descriptor,
    ) = inputs;
    final optedIn =
        Env.pushGatewayConfigured &&
        community?.pushNotificationsEnabled == true &&
        _signedIn(config, community, memberPubkey);
    final shouldRegister =
        session.status == SessionStatus.connected &&
        optedIn &&
        descriptor != null;
    // Read by the resume listener, which outlives a build. A resume often
    // lands while the relay reconnects, when its descriptor is gone, and the
    // registration attempt after reconnecting reuses its key, so it would not
    // refresh the token either. Remember instead whether this community's
    // relay advertised push, so a relay without it never fetches a token.
    final pushRelay = useRef<String?>(null);
    if (descriptor != null) pushRelay.value = community?.id;
    final registering = useRef(false);
    registering.value = optedIn && pushRelay.value == community?.id;
    final enablementEpoch = useRef(0);
    useEffect(() {
      // Each opt-in needs a fresh active lease even when endpoint, executor,
      // and subscriptions match the lease replaced by the opt-out tombstone.
      // A distinct key also fences renewal callbacks from the prior opt-in.
      if (community?.pushNotificationsEnabled == true) {
        enablementEpoch.value += 1;
      }
      return null;
    }, [community?.id, community?.pushNotificationsEnabled]);

    useEffect(() {
      final listener = AppLifecycleListener(
        onResume: () {
          _runRevocationOutbox(revocationOutbox.trigger);
          // FCM rotates its token without telling a stopped app. Only a phone
          // that registers for push has one to refresh.
          if (!registering.value) return;
          unawaited(
            refreshBuzzPushToken().catchError((
              Object error,
              StackTrace stackTrace,
            ) {
              debugPrint('FCM token refresh failed: $error');
            }),
          );
        },
      );
      _runRevocationOutbox(revocationOutbox.start);
      return listener.dispose;
    }, [revocationOutbox]);

    useEffect(() {
      if (session.status == SessionStatus.connected) {
        _runRevocationOutbox(revocationOutbox.trigger);
      }
      return null;
    }, [revocationOutbox, session.status]);

    useEffect(
      () => () {
        publicationAttempt.dispose();
        tombstoneAttempt.dispose();
        reconcileAttempt.dispose();
      },
      const [],
    );

    useEffect(
      () {
        final pendingCommunities = communities
            .where(
              (candidate) =>
                  !candidate.pushNotificationsEnabled &&
                  candidate.pushSubscriptionState.pendingTombstoneGeneration !=
                      null,
            )
            .toList();
        if (session.status != SessionStatus.connected ||
            pendingCommunities.isEmpty) {
          return null;
        }
        final attempt = buzzPushTombstoneAttemptKey(pendingCommunities);
        if (!tombstoneAttempt.tryBegin(attempt)) return null;
        unawaited(() async {
          try {
            Object? firstError;
            StackTrace? firstStack;
            for (final pendingCommunity in pendingCommunities) {
              try {
                await ref
                    .read(communityListProvider.notifier)
                    .retryPendingPushLeaseTombstone(
                      pendingCommunity.id,
                      advanceGeneration: true,
                    );
              } catch (error, stack) {
                firstError ??= error;
                firstStack ??= stack;
              }
            }
            if (firstError != null) {
              Error.throwWithStackTrace(firstError, firstStack!);
            }
            tombstoneAttempt.complete(attempt);
          } catch (error, stack) {
            tombstoneAttempt.failed(
              attempt,
              retry: () {
                if (context.mounted) tombstoneRetry.value += 1;
              },
            );
            debugPrint('Push lease tombstone retry failed: $error');
            debugPrintStack(stackTrace: stack);
          }
        }());
        return null;
      },
      [
        session.status,
        for (final candidate in communities)
          '${candidate.id}|${candidate.pushNotificationsEnabled}|'
              '${candidate.pushSubscriptionState.pendingTombstoneGeneration}',
        tombstoneRetry.value,
      ],
    );

    // Removing the last community with push already off leaves the relay set
    // unchanged (empty), but it is what makes the installation revocable.
    final noCommunities = communities.isEmpty;
    final pushRelayUrls = [
      for (final candidate in communities)
        if (candidate.pushNotificationsEnabled) candidate.relayUrl,
    ];
    useEffect(
      () {
        // Authority the phone holds for relays no community uses is revoked:
        // a community whose push is off, or removed, and at last the whole
        // installation. A list still loading looks empty, so wait for it.
        if (!communitiesLoaded ||
            !Env.pushGatewayConfigured ||
            defaultTargetPlatform != TargetPlatform.android) {
          return null;
        }
        final attempt =
            '${_pushRelayOrigins(communities).join('|')}|$noCommunities';
        if (!reconcileAttempt.tryBegin(attempt)) return null;
        unawaited(() async {
          try {
            await reconcileBuzzPushAuthority(
              desiredRelays: () => _desiredPushRelays(ref),
              noCommunities: () =>
                  (ref.read(communityListProvider).value ?? const []).isEmpty,
            );
            reconcileAttempt.complete(attempt);
          } catch (error, stack) {
            reconcileAttempt.failed(
              attempt,
              retry: () {
                if (context.mounted) reconcileRetry.value += 1;
              },
            );
            reportPushLeaseCleanupError(error, stack);
          }
        }());
        return null;
      },
      [
        communitiesLoaded,
        pushRelayUrls.join('|'),
        noCommunities,
        reconcileRetry.value,
      ],
    );

    final token = apnsDeviceToken.value ?? fcmDeviceToken.value;
    useEffect(
      () {
        if (!shouldRegister || token == null) {
          return null;
        }
        final activeCommunity = community!;
        final activeDescriptor = descriptor;
        final state = activeCommunity.pushSubscriptionState;
        if (state.desired.isEmpty) return null;
        final publicationKey = buzzPushPublicationAttemptKey(
          communityId: activeCommunity.id,
          relayBaseUrl: config.baseUrl,
          token: token,
          descriptor: activeDescriptor,
          subscriptions: state.desired,
        );
        final attempt = '${enablementEpoch.value}|$publicationKey';
        if (!publicationAttempt.tryBegin(attempt)) return null;
        final relay = SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: config.nsec!,
        );
        unawaited(() async {
          try {
            final grant = await _publish(
              ref,
              config,
              activeCommunity,
              memberPubkey!,
              relay,
            );
            if (grant == null) {
              // Push was turned off, or the community removed, while this
              // waited. Nothing was published, and the opt-out already ran.
              publicationAttempt.complete(attempt);
              return;
            }
            final renewInMilliseconds =
                grant.expiresAt * 1000 -
                DateTime.now().millisecondsSinceEpoch -
                const Duration(minutes: 5).inMilliseconds;
            publicationAttempt.retryAfter(
              attempt,
              delay: Duration(
                milliseconds: renewInMilliseconds > 1000
                    ? renewInMilliseconds
                    : 1000,
              ),
              retry: () {
                if (context.mounted) publicationRetry.value += 1;
              },
            );
          } catch (error, stack) {
            void retry() {
              if (context.mounted) publicationRetry.value += 1;
            }

            if (error is AndroidPushAttestationUnavailable) {
              // Only a different boot state or gateway configuration changes
              // this, so look again rarely rather than backing off briefly.
              publicationAttempt.retryAfter(
                attempt,
                delay: const Duration(hours: 1),
                retry: retry,
              );
            } else if (error is BuzzPushChannelGrammarException) {
              // Re-fetch capabilities after a relay upgrade without repeatedly
              // enrolling or publishing a lease the old relay cannot accept.
              publicationAttempt.retryAfter(
                attempt,
                delay: const Duration(minutes: 5),
                retry: retry,
              );
            } else {
              publicationAttempt.failed(attempt, retry: retry);
            }
            debugPrint('Push lease bootstrap failed: $error');
            debugPrintStack(stackTrace: stack);
          }
        }());
        return null;
      },
      [
        session.status,
        config.baseUrl,
        community?.id,
        community?.pushNotificationsEnabled,
        community?.pushSubscriptionState,
        memberPubkey,
        descriptor,
        token,
        publicationRetry.value,
      ],
    );

    return BuzzPushRegistrationBootstrap(
      shouldRegister: shouldRegister,
      attemptKey: '${community?.id}|${config.baseUrl}',
      child: const SizedBox.shrink(),
    );
  }

  static bool _signedIn(
    RelayConfig config,
    Community? community,
    String? memberPubkey,
  ) =>
      community != null &&
      config.nsec != null &&
      config.nsec!.isNotEmpty &&
      memberPubkey != null &&
      memberPubkey.isNotEmpty;

  /// Returns null when the community stopped wanting push before the lease was
  /// published.
  static Future<BuzzPushEndpointGrant?> _publish(
    WidgetRef ref,
    RelayConfig config,
    Community community,
    String memberPubkey,
    SignedEventRelay relay,
  ) async {
    final state = community.pushSubscriptionState;
    final desired = state.desired;
    final descriptor = await ref.read(buzzPushDescriptorFetcherProvider)(
      config.baseUrl,
    );
    validateBuzzPushChannelGrammar(descriptor, desired);
    // The opt-out's revocation may already have run while the descriptor was
    // fetched or enrollment waited its turn, so enrolling or publishing now
    // would create authority nothing revokes. Asked again before each step.
    bool stillWanted() => (ref.read(communityListProvider).value ?? const [])
        .any((c) => c.id == community.id && c.pushNotificationsEnabled);
    final grant = await enrollBuzzPush(
      config.wsUrl,
      Env.pushGatewayUrl,
      communitiesForSnapshotRefresh:
          ref.read(communityListProvider).value ?? [community],
      relayOrigin: descriptor.origin,
      relayPubkey: descriptor.executorPubkey,
      desiredRelays: () => _desiredPushRelays(ref),
      stillWanted: stillWanted,
      // Enrollment can finish after a community switch, when reports already
      // name the next member.
      diagnostics: SignerDiagnosticReporter(
        ref.read(diagnosticReporterProvider),
        signer: memberPubkey,
      ),
    );
    if (grant == null || !stillWanted()) return null;
    // Relay lease replacement and gateway delegation are independent state
    // machines. Subscription changes advance only the kind-30350 generation;
    // the opaque grant remains reusable until its own authority changes.
    final notifier = ref.read(communityListProvider.notifier);
    await publishBuzzPushLeaseRecoverably(
      reserveGeneration: () =>
          notifier.reservePushLeaseGeneration(community.id),
      publish: (leaseGeneration) => publishBuzzDevPushLeaseThroughRelay(
        grant: grant,
        leaseInstallationId: community.pushLeaseInstallationId,
        leaseGeneration: leaseGeneration,
        descriptor: descriptor,
        nsec: config.nsec!,
        memberPubkey: memberPubkey,
        subscriptions: desired,
        relay: relay,
      ),
      markAccepted: (leaseGeneration) => notifier.markPushLeaseAccepted(
        community.id,
        subscriptions: desired,
        generation: leaseGeneration,
      ),
    );
    return grant;
  }
}

Set<String> _desiredPushRelays(WidgetRef ref) =>
    _pushRelayOrigins(ref.read(communityListProvider).value ?? const []);

/// The relays of the communities whose push is on, as grants name them. A
/// relay URL with no origin (a path, say) never had a grant, so it names none.
Set<String> _pushRelayOrigins(List<Community> communities) {
  final origins = <String>{};
  for (final community in communities) {
    if (!community.pushNotificationsEnabled) continue;
    try {
      origins.add(canonicalBuzzPushRelayOrigin(community.relayUrl));
    } on FormatException {
      continue;
    }
  }
  return origins;
}

void _runRevocationOutbox(Future<void> Function() operation) {
  unawaited(
    operation().catchError((Object error, StackTrace stackTrace) {
      reportPushLeaseCleanupError(error, stackTrace);
    }),
  );
}
