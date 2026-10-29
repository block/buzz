import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community.dart';
import '../community/community_provider.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import '../relay/signed_event_relay.dart';
import 'dev_push_lease.dart';
import 'push_bridge.dart';
import 'push_lease_revocation_outbox.dart';
import 'push_relay_capability_provider.dart';
import 'push_subscription.dart';
import 'push_subscription_provider.dart';
import 'push_setup_recovery.dart';

const _pushBootstrapRetryDelay = Duration(seconds: 5);

@visibleForTesting
class BuzzPushAttemptGate {
  BuzzPushAttemptGate({this.retryDelay = _pushBootstrapRetryDelay});

  final Duration retryDelay;
  String? _attempt;
  Timer? _retryTimer;

  bool tryBegin(String attempt) {
    if (_attempt == attempt) return false;
    _retryTimer?.cancel();
    _retryTimer = null;
    _attempt = attempt;
    return true;
  }

  void failed(String attempt, {required VoidCallback retry}) {
    if (_attempt != attempt) return;
    _retryTimer?.cancel();
    _retryTimer = Timer(retryDelay, () {
      _retryTimer = null;
      if (_attempt != attempt) return;
      _attempt = null;
      retry();
    });
  }

  void retryAfter(
    String attempt, {
    required Duration delay,
    required VoidCallback retry,
  }) {
    if (_attempt != attempt) return;
    _retryTimer?.cancel();
    _retryTimer = Timer(delay, () {
      _retryTimer = null;
      if (_attempt != attempt) return;
      _attempt = null;
      retry();
    });
  }

  void complete(String attempt) {
    if (_attempt != attempt) return;
    _retryTimer?.cancel();
    _retryTimer = null;
    _attempt = null;
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

/// Restores attempted leases across enabled relay origins after token rotation.
/// Retrying includes nonempty grants because native save precedes relay handoff.
@visibleForTesting
Future<int?> renewBuzzPushCommunityLeases({
  required List<Community> Function() readCommunities,
  required List<BuzzPushEndpointGrant> grants,
  required Future<BuzzPushEndpointGrant> Function(Community community) publish,
}) async {
  Object? firstError;
  StackTrace? firstStack;
  int? expiresAt;
  final origins = {for (final grant in grants) grant.relayOrigin};
  for (final id
      in readCommunities().map((community) => community.id).toList()) {
    final community = readCommunities()
        .where((value) => value.id == id)
        .firstOrNull;
    if (community == null ||
        !community.pushNotificationsEnabled ||
        community.pushSubscriptionState.desired.isEmpty ||
        community.nsec == null ||
        community.nsec!.isEmpty) {
      continue;
    }
    try {
      if (!origins.contains(canonicalBuzzPushRelayOrigin(community.relayUrl))) {
        continue;
      }
      final grant = await publish(community);
      if (expiresAt == null || grant.expiresAt < expiresAt) {
        expiresAt = grant.expiresAt;
      }
    } catch (error, stack) {
      firstError ??= error;
      firstStack ??= stack;
    }
  }
  if (firstError != null) Error.throwWithStackTrace(firstError, firstStack!);
  return expiresAt;
}

typedef _PushInputs = ({
  BuzzPushLeaseRevocationOutbox revocationOutbox,
  SessionState session,
  List<Community> communities,
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
    useListenable(pushEndpointGrants);
    final publicationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final publicationBusy = useRef(false);
    final publishedGrant = useRef<BuzzPushEndpointGrant?>(null);
    final sharedAuthority = useRef('');
    final tombstoneAttempt = useMemoized(BuzzPushAttemptGate.new);
    final otherOriginsAttempt = useMemoized(BuzzPushAttemptGate.new);
    final otherOriginsBusy = useRef(false);
    final latestOtherOriginsAttempt = useRef('');
    final otherOriginsRetry = useState(0);
    final publicationRetry = useState(0);
    final tombstoneRetry = useState(0);
    final (
      :revocationOutbox,
      :session,
      :communities,
      :config,
      :community,
      :memberPubkey,
      :descriptor,
    ) = inputs;
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
        onResume: () => _runRevocationOutbox(revocationOutbox.trigger),
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
        otherOriginsAttempt.dispose();
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
        const attempt = 'pending-tombstones';
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

    final token = apnsDeviceToken.value;
    final activeSelection = ref.watch(activeCommunityProvider);
    final nativeCleanupKey = [
      for (final grant in pushEndpointGrants.value)
        '${grant.relayOrigin}|${grant.relayPubkey}|${grant.gatewayInstallationHandle}|'
            '${grant.generation}|${grant.endpointEpoch}|${grant.endpointGrant.isEmpty}|${grant.pending}',
    ].join(';');
    final allOriginsKey =
        '$token|$nativeCleanupKey|${activeSelection.hasValue}|${activeSelection.value?.id}|${[for (final value in communities) '${value.id}|${value.pushNotificationsEnabled}|'
              '${buzzPushSubscriptionsFingerprint(value.pushSubscriptionState.desired)}'].join(';')}';
    latestOtherOriginsAttempt.value = allOriginsKey;
    useEffect(() {
      if (!Env.pushGatewayConfigured ||
          otherOriginsBusy.value ||
          !otherOriginsAttempt.tryBegin(allOriginsKey)) {
        return null;
      }
      otherOriginsBusy.value = true;
      unawaited(() async {
        var succeeded = false;
        try {
          if (!ref.read(communityListProvider).hasValue) {
            throw StateError('Push community storage is still loading.');
          }
          final grants = await readBuzzPushEndpointGrants();
          if (!context.mounted) return;
          await revocationOutbox.reconcileGatewayCleanup(
            ref.read(communityListProvider).value!,
            readGrants: () async => grants,
          );
          final expiresAt = token == null
              ? null
              : await renewBuzzPushCommunityLeases(
                  readCommunities: () {
                    if (!context.mounted) return const [];
                    final active = ref.read(activeCommunityProvider);
                    if (!active.hasValue) return const [];
                    return (ref.read(communityListProvider).value ?? const [])
                        .where((value) => value.id != active.value?.id)
                        .toList();
                  },
                  grants: grants,
                  publish: (value) {
                    final member = value.pubkey ?? pubkeyFromNsec(value.nsec);
                    if (member == null) {
                      throw StateError('Push lease signing key is missing.');
                    }
                    return _publish(
                      ref,
                      RelayConfig(baseUrl: value.relayUrl, nsec: value.nsec),
                      value,
                      member,
                      null,
                    );
                  },
                );
          succeeded = true;
          if (expiresAt != null && context.mounted) {
            final renewIn =
                expiresAt * 1000 -
                DateTime.now().millisecondsSinceEpoch -
                const Duration(minutes: 5).inMilliseconds;
            otherOriginsAttempt.retryAfter(
              allOriginsKey,
              delay: Duration(milliseconds: renewIn > 1000 ? renewIn : 1000),
              retry: () {
                if (context.mounted) otherOriginsRetry.value += 1;
              },
            );
          }
        } catch (error, stack) {
          if (context.mounted) {
            otherOriginsAttempt.failed(
              allOriginsKey,
              retry: () {
                if (context.mounted) otherOriginsRetry.value += 1;
              },
            );
          }
          reportPushSetupError('relay lease renewal', error, stack);
        } finally {
          otherOriginsBusy.value = false;
          if (succeeded &&
              context.mounted &&
              latestOtherOriginsAttempt.value != allOriginsKey) {
            otherOriginsAttempt.complete(allOriginsKey);
            otherOriginsRetry.value += 1;
          }
        }
      }());
      return null;
    }, [allOriginsKey, otherOriginsRetry.value]);

    useEffect(
      () {
        if (!_ready(session, config, community, memberPubkey) ||
            !buzzPushLifecycleEnabled(
              community: community,
              descriptor: descriptor,
            ) ||
            token == null) {
          return null;
        }
        final activeCommunity = community!;
        final activeDescriptor = descriptor!;
        final state = activeCommunity.pushSubscriptionState;
        if (state.desired.isEmpty || publicationBusy.value) return null;
        // Compare with the last completed handoff, after coalescing notifications
        // received during publication. Only another origin's newer authority
        // changes the attempt; our own issuance must not trigger a renewal loop.
        final previous = publishedGrant.value;
        if (previous != null &&
            previous.gatewayInstallationHandle?.isNotEmpty == true &&
            previous.relayOrigin ==
                canonicalBuzzPushRelayOrigin(config.baseUrl)) {
          var newest = previous;
          for (final candidate in pushEndpointGrants.value) {
            if (candidate.pending ||
                candidate.endpointGrant.isEmpty ||
                candidate.relayOrigin == previous.relayOrigin ||
                candidate.gatewayInstallationHandle !=
                    previous.gatewayInstallationHandle ||
                candidate.relayPubkey != previous.relayPubkey) {
              continue;
            }
            if (candidate.endpointEpoch > newest.endpointEpoch ||
                (candidate.endpointEpoch == newest.endpointEpoch &&
                    candidate.generation > newest.generation)) {
              newest = candidate;
            }
          }
          if (newest.endpointEpoch != previous.endpointEpoch ||
              newest.generation != previous.generation) {
            sharedAuthority.value =
                '${newest.gatewayInstallationHandle}|'
                '${newest.relayPubkey}|${newest.endpointEpoch}|${newest.generation}';
          }
        }
        final publicationKey = buzzPushPublicationAttemptKey(
          communityId: activeCommunity.id,
          relayBaseUrl: config.baseUrl,
          token: token,
          descriptor: activeDescriptor,
          subscriptions: state.desired,
        );
        final attempt =
            '${enablementEpoch.value}|$publicationKey|${sharedAuthority.value}';
        if (!publicationAttempt.tryBegin(attempt)) return null;
        publicationBusy.value = true;
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
            if (!context.mounted) return;
            publishedGrant.value = grant;
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
            publicationAttempt.retryAfter(
              attempt,
              // Re-fetch capabilities after a relay upgrade without repeatedly
              // enrolling or publishing a lease the old relay cannot accept.
              delay: error is BuzzPushChannelGrammarException
                  ? const Duration(minutes: 5)
                  : _pushBootstrapRetryDelay,
              retry: () {
                if (context.mounted) publicationRetry.value += 1;
              },
            );
            debugPrint('Push lease bootstrap failed: $error');
            debugPrintStack(stackTrace: stack);
          } finally {
            publicationBusy.value = false;
            // Recheck inputs/authority that changed while the handoff was pending.
            if (context.mounted) publicationRetry.value += 1;
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
        nativeCleanupKey,
        publicationRetry.value,
      ],
    );

    return BuzzPushRegistrationBootstrap(
      shouldRegister:
          _ready(session, config, community, memberPubkey) &&
          buzzPushLifecycleEnabled(
            community: community,
            descriptor: descriptor,
          ),
      attemptKey: '${community?.id}|${config.baseUrl}',
      child: const SizedBox.shrink(),
    );
  }

  static bool _ready(
    SessionState session,
    RelayConfig config,
    Community? community,
    String? memberPubkey,
  ) =>
      session.status == SessionStatus.connected &&
      community != null &&
      config.nsec != null &&
      config.nsec!.isNotEmpty &&
      memberPubkey != null &&
      memberPubkey.isNotEmpty;

  static Future<BuzzPushEndpointGrant> _publish(
    WidgetRef ref,
    RelayConfig config,
    Community community,
    String memberPubkey,
    SignedEventRelay? relay,
  ) async {
    final state = community.pushSubscriptionState;
    final desired = state.desired;
    final descriptor = await ref.read(buzzPushDescriptorFetcherProvider)(
      config.baseUrl,
    );
    validateBuzzPushChannelGrammar(descriptor, desired);
    bool currentIntent() {
      if (!ref.context.mounted) return false;
      final current = ref
          .read(communityListProvider)
          .value
          ?.where((value) => value.id == community.id)
          .firstOrNull;
      return current != null &&
          current.pushNotificationsEnabled &&
          current.nsec?.trim() == config.nsec &&
          current.relayUrl == community.relayUrl &&
          buzzPushSubscriptionsFingerprint(
                current.pushSubscriptionState.desired,
              ) ==
              buzzPushSubscriptionsFingerprint(desired);
    }

    if (!currentIntent()) throw StateError('Push setup was superseded.');

    late final BuzzPushEndpointGrant grant;
    try {
      grant = await enrollBuzzPush(
        config.wsUrl,
        Env.pushGatewayUrl,
        communitiesForSnapshotRefresh:
            ref.read(communityListProvider).value ?? [community],
      );
    } catch (error, stack) {
      try {
        await readBuzzPushEndpointGrants();
      } catch (readError, readStack) {
        reportPushSetupError('pending gateway cleanup', readError, readStack);
      }
      Error.throwWithStackTrace(error, stack);
    }
    if (!currentIntent()) {
      if (ref.context.mounted && ref.read(communityListProvider).hasValue) {
        await ref
            .read(buzzPushLeaseRevocationOutboxProvider)
            .reconcileGatewayCleanup(ref.read(communityListProvider).value!);
      }
      throw StateError('Push setup was superseded.');
    }
    // Relay lease replacement and gateway delegation are independent state
    // machines. Subscription changes advance only the kind-30350 generation;
    // the opaque grant remains reusable until its own authority changes.
    final notifier = ref.read(communityListProvider.notifier);
    await publishBuzzPushLeaseRecoverably(
      reserveGeneration: () =>
          notifier.reservePushLeaseGeneration(community.id),
      publish: (leaseGeneration) => publishBuzzDevPushLease(
        grant: grant,
        leaseInstallationId: community.pushLeaseInstallationId,
        leaseGeneration: leaseGeneration,
        descriptor: descriptor,
        nsec: config.nsec!,
        memberPubkey: memberPubkey,
        subscriptions: desired,
        submit:
            relay?.submit ??
            ({required kind, required content, required tags, createdAt}) =>
                submitSignedEventOnce(
                  wsUrl: config.wsUrl,
                  nsec: config.nsec!,
                  kind: kind,
                  content: content,
                  tags: tags,
                  createdAt: createdAt,
                ),
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

void _runRevocationOutbox(Future<void> Function() operation) {
  unawaited(
    operation().catchError((Object error, StackTrace stackTrace) {
      reportPushLeaseCleanupError(error, stackTrace);
    }),
  );
}
