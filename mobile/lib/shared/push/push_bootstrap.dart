import 'dart:async';

import 'package:flutter/foundation.dart';
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

const _pushBootstrapRetryDelay = Duration(seconds: 5);
const _maxGatewayInitializationRetries = 6;

@visibleForTesting
Duration? buzzPushGatewayInitializationRetryDelay(int failureCount) {
  if (failureCount < 1) {
    throw ArgumentError.value(failureCount, 'failureCount', 'must be positive');
  }
  if (failureCount > _maxGatewayInitializationRetries) return null;
  return Duration(
    seconds: _pushBootstrapRetryDelay.inSeconds << (failureCount - 1),
  );
}

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

  bool isCurrent(String attempt) => _attempt == attempt;

  void failed(String attempt, {required VoidCallback retry}) {
    if (_attempt != attempt) return;
    _attempt = null;
    _retryTimer?.cancel();
    _retryTimer = Timer(retryDelay, () {
      _retryTimer = null;
      if (_attempt == null) retry();
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
class BuzzPushAttemptFailureBudget {
  String? _attempt;
  int _failureCount = 0;

  int recordFailure(String attempt) {
    if (_attempt != attempt) {
      _attempt = attempt;
      _failureCount = 0;
    }
    return ++_failureCount;
  }

  void clear(String attempt) {
    if (_attempt != attempt) return;
    _attempt = null;
    _failureCount = 0;
  }
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
  buzzPushSubscriptionsFingerprint(subscriptions),
].join('|');

@visibleForTesting
bool buzzPushLifecycleEnabled({
  required Community? community,
  required BuzzPushLeaseDescriptor? descriptor,
}) => community?.pushNotificationsEnabled == true && descriptor != null;

@visibleForTesting
List<Community> buzzPushCommunitiesRequiringGatewayMigration({
  required List<Community> communities,
  required Set<String> retiredRelayOrigins,
  Set<String> replacementRelayOrigins = const {},
  required String targetGatewayOrigin,
}) => communities
    .where(
      (community) =>
          community.pushNotificationsEnabled &&
          retiredRelayOrigins.contains(
            buzzPushRelayWebSocketOrigin(community.relayUrl),
          ) &&
          (replacementRelayOrigins.contains(
                buzzPushRelayWebSocketOrigin(community.relayUrl),
              ) ||
              community.pushSubscriptionState.acceptedGatewayOrigin !=
                  targetGatewayOrigin),
    )
    .toList();

@visibleForTesting
bool buzzPushGatewayMigrationAttemptIsCurrent({
  required bool attemptIsCurrent,
  required String token,
  required String? liveToken,
  required Set<String> retiredRelayOrigins,
  required Set<String> liveRetiredRelayOrigins,
  required Set<String> replacementRelayOrigins,
  required Set<String> liveReplacementRelayOrigins,
  required int replacementGeneration,
  required int liveReplacementGeneration,
}) =>
    attemptIsCurrent &&
    token == liveToken &&
    setEquals(retiredRelayOrigins, liveRetiredRelayOrigins) &&
    setEquals(replacementRelayOrigins, liveReplacementRelayOrigins) &&
    replacementGeneration == liveReplacementGeneration;

@visibleForTesting
Future<bool> markBuzzPushGatewayMigrationAcceptedIfCurrent({
  required bool Function() attemptIsCurrent,
  required Future<bool> Function() markAccepted,
}) {
  if (!attemptIsCurrent()) return Future.value(false);
  return markAccepted();
}

@visibleForTesting
Future<T> runBuzzPushGatewayMigrationMutationIfCurrent<T>({
  required bool Function() attemptIsCurrent,
  required Future<T> Function() mutate,
}) {
  if (!attemptIsCurrent()) {
    return Future.error(
      StateError('Push gateway migration attempt is obsolete.'),
    );
  }
  return mutate();
}

typedef BuzzPushGatewayMigrationTarget = ({
  Community community,
  String relayOrigin,
  BuzzPushLeaseDescriptor descriptor,
});

@visibleForTesting
class BuzzPushGatewayMigrationResolution {
  BuzzPushGatewayMigrationResolution({
    required this.targets,
    required this.blockedOrigins,
    this.firstError,
    this.firstStack,
  });

  final List<BuzzPushGatewayMigrationTarget> targets;
  final Set<String> blockedOrigins;
  final Object? firstError;
  final StackTrace? firstStack;

  void throwIfFailed() {
    if (firstError != null) {
      Error.throwWithStackTrace(firstError!, firstStack!);
    }
  }
}

@visibleForTesting
Future<BuzzPushGatewayMigrationResolution>
resolveBuzzPushGatewayMigrationTargets({
  required Iterable<Community> communities,
  required Future<BuzzPushLeaseDescriptor> Function(String relayUrl)
  fetchDescriptor,
}) async {
  final ordered = communities.toList()
    ..sort(
      (left, right) => buzzPushRelayWebSocketOrigin(
        left.relayUrl,
      ).compareTo(buzzPushRelayWebSocketOrigin(right.relayUrl)),
    );
  final targets = <BuzzPushGatewayMigrationTarget>[];
  final blockedOrigins = <String>{};
  Object? firstError;
  StackTrace? firstStack;
  for (final community in ordered) {
    final relayOrigin = buzzPushRelayWebSocketOrigin(community.relayUrl);
    try {
      targets.add((
        community: community,
        relayOrigin: relayOrigin,
        descriptor: await fetchDescriptor(community.relayUrl),
      ));
    } catch (error, stack) {
      blockedOrigins.add(relayOrigin);
      firstError ??= error;
      firstStack ??= stack;
    }
  }
  return BuzzPushGatewayMigrationResolution(
    targets: targets,
    blockedOrigins: blockedOrigins,
    firstError: firstError,
    firstStack: firstStack,
  );
}

@visibleForTesting
Map<String, List<BuzzPushGatewayMigrationTarget>>
buzzPushGroupGatewayMigrationsByDelegationAuthority(
  Iterable<BuzzPushGatewayMigrationTarget> targets,
) {
  final groups = <String, List<BuzzPushGatewayMigrationTarget>>{};
  for (final target in targets) {
    groups.putIfAbsent(target.descriptor.executorPubkey, () => []).add(target);
  }
  return groups;
}

@visibleForTesting
Set<String> buzzPushGatewayMigrationGroupOriginsToQueue({
  required Iterable<BuzzPushGatewayMigrationTarget> targets,
  required Set<String> replacementRelayOrigins,
}) {
  final groupOrigins = targets.map((target) => target.relayOrigin).toSet();
  if (groupOrigins.intersection(replacementRelayOrigins).isEmpty ||
      replacementRelayOrigins.containsAll(groupOrigins)) {
    return const {};
  }
  return groupOrigins;
}

@visibleForTesting
Future<bool> processBuzzPushGatewayMigrationGroups<T>({
  required Iterable<T> groups,
  required Future<bool> Function(T group) process,
  Object? initialError,
  StackTrace? initialStack,
}) async {
  Object? firstError = initialError;
  StackTrace? firstStack = initialStack;
  for (final group in groups) {
    try {
      if (await process(group)) return true;
    } catch (error, stack) {
      firstError ??= error;
      firstStack ??= stack;
    }
  }
  if (firstError != null) {
    Error.throwWithStackTrace(firstError, firstStack!);
  }
  return false;
}

/// Owns the APNs-registration side effect so migration-triggered registration
/// is exercised through the same production boundary as active-community
/// registration.
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
String buzzPushRelayWebSocketOrigin(String relayUrl) {
  final uri = Uri.parse(RelayConfig(baseUrl: relayUrl).wsUrl);
  return uri.replace(path: '', query: null, fragment: null).toString();
}

@visibleForTesting
String buzzPushGatewayOrigin(String gatewayUrl) {
  final uri = Uri.parse(gatewayUrl);
  return uri.replace(path: '', query: null, fragment: null).toString();
}

@visibleForTesting
Future<int> publishBuzzPushLeaseRecoverably({
  required Future<int> Function() reserveGeneration,
  required Future<void> Function(int generation) publish,
  required Future<bool> Function(int generation) markAccepted,
  bool Function()? operationIsCurrent,
}) async {
  if (operationIsCurrent?.call() == false) {
    throw StateError('Push lease publication attempt is obsolete.');
  }
  final generation = await reserveGeneration();
  if (operationIsCurrent?.call() == false) {
    throw StateError('Push lease publication attempt is obsolete.');
  }
  await publish(generation);
  if (!await markAccepted(generation)) {
    throw StateError('A newer push lease superseded the published generation.');
  }
  return generation;
}

/// Starts the push lifecycle only after authenticated relay connectivity and a
/// push-capable NIP-11 descriptor are both present.
class BuzzPushBootstrap extends HookConsumerWidget {
  const BuzzPushBootstrap({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(apnsDeviceToken);
    useListenable(retiredBuzzPushRelayOrigins);
    useListenable(replacementBuzzPushRelayOrigins);
    useListenable(replacementBuzzPushGeneration);
    final gatewayInitializationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final publicationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final gatewayMigrationAttempt = useMemoized(BuzzPushAttemptGate.new);
    final tombstoneAttempt = useMemoized(BuzzPushAttemptGate.new);
    final gatewayInitializationRetry = useState(0);
    final gatewayInitializationFailures = useRef(0);
    final publicationRetry = useState(0);
    final gatewayMigrationRetry = useState(0);
    final gatewayMigrationFailures = useMemoized(
      BuzzPushAttemptFailureBudget.new,
    );
    final tombstoneRetry = useState(0);
    final revocationOutbox = ref.watch(buzzPushLeaseRevocationOutboxProvider);
    final session = ref.watch(relaySessionProvider);
    final communitiesAsync = ref.watch(communityListProvider);
    final communities = communitiesAsync.value ?? const [];
    final config = ref.watch(relayConfigProvider);
    final community = ref.watch(activeCommunityProvider).value;
    final memberPubkey = ref.watch(myPubkeyProvider);
    final descriptor = ref.watch(currentRelayPushDescriptorProvider).value;
    final token = apnsDeviceToken.value;
    final retiredRelayOrigins = retiredBuzzPushRelayOrigins.value;
    final replacementRelayOrigins = replacementBuzzPushRelayOrigins.value;
    final replacementGeneration = replacementBuzzPushGeneration.value;
    final migrationRelayOrigins = retiredRelayOrigins.union(
      replacementRelayOrigins,
    );
    final targetGatewayOrigin = buzzPushGatewayOrigin(Env.pushGatewayUrl);
    final migrationCommunities = buzzPushCommunitiesRequiringGatewayMigration(
      communities: communities,
      retiredRelayOrigins: migrationRelayOrigins,
      replacementRelayOrigins: replacementRelayOrigins,
      targetGatewayOrigin: targetGatewayOrigin,
    );
    final activeLifecycleReady =
        _ready(session, config, community, memberPubkey) &&
        buzzPushLifecycleEnabled(community: community, descriptor: descriptor);

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

    useEffect(() {
      const attempt = 'configured-gateway';
      if (!gatewayInitializationAttempt.tryBegin(attempt)) return null;
      unawaited(() async {
        try {
          await initializeBuzzPushGateway();
          gatewayInitializationFailures.value = 0;
          gatewayInitializationAttempt.complete(attempt);
        } catch (error, stack) {
          gatewayInitializationFailures.value += 1;
          final retryDelay = buzzPushGatewayInitializationRetryDelay(
            gatewayInitializationFailures.value,
          );
          if (retryDelay == null) {
            gatewayInitializationAttempt.complete(attempt);
          } else {
            gatewayInitializationAttempt.retryAfter(
              attempt,
              delay: retryDelay,
              retry: () {
                if (context.mounted) gatewayInitializationRetry.value += 1;
              },
            );
          }
          debugPrint('Push gateway initialization failed: $error');
          if (retryDelay == null) {
            debugPrint(
              'Push gateway migration is deferred until the next app launch.',
            );
          }
          debugPrintStack(stackTrace: stack);
        }
      }());
      return null;
    }, [gatewayInitializationRetry.value]);

    useEffect(
      () => () {
        gatewayInitializationAttempt.dispose();
        publicationAttempt.dispose();
        gatewayMigrationAttempt.dispose();
        tombstoneAttempt.dispose();
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

    final activeCommunityAwaitingGatewayMigration =
        community != null &&
        buzzPushCommunitiesRequiringGatewayMigration(
          communities: [community],
          retiredRelayOrigins: migrationRelayOrigins,
          replacementRelayOrigins: replacementRelayOrigins,
          targetGatewayOrigin: targetGatewayOrigin,
        ).isNotEmpty;
    useEffect(
      () {
        if (token == null ||
            migrationRelayOrigins.isEmpty ||
            !communitiesAsync.hasValue) {
          return null;
        }
        final attempt = [
          token,
          'retired:${(retiredRelayOrigins.toList()..sort()).join(',')}',
          'replacement:${(replacementRelayOrigins.toList()..sort()).join(',')}',
          'replacement-generation:$replacementGeneration',
        ].join('|');
        if (!gatewayMigrationAttempt.tryBegin(attempt)) return null;
        unawaited(() async {
          bool attemptIsCurrent() => buzzPushGatewayMigrationAttemptIsCurrent(
            attemptIsCurrent: gatewayMigrationAttempt.isCurrent(attempt),
            token: token,
            liveToken: apnsDeviceToken.value,
            retiredRelayOrigins: retiredRelayOrigins,
            liveRetiredRelayOrigins: retiredBuzzPushRelayOrigins.value,
            replacementRelayOrigins: replacementRelayOrigins,
            liveReplacementRelayOrigins: replacementBuzzPushRelayOrigins.value,
            replacementGeneration: replacementGeneration,
            liveReplacementGeneration: replacementBuzzPushGeneration.value,
          );
          try {
            final candidates = buzzPushCommunitiesRequiringGatewayMigration(
              communities: communities,
              retiredRelayOrigins: migrationRelayOrigins,
              replacementRelayOrigins: replacementRelayOrigins,
              targetGatewayOrigin: targetGatewayOrigin,
            );
            final resolution = await resolveBuzzPushGatewayMigrationTargets(
              communities: candidates,
              fetchDescriptor: fetchBuzzPushLeaseDescriptor,
            );
            final authorityGroups =
                buzzPushGroupGatewayMigrationsByDelegationAuthority(
                  resolution.targets,
                );
            final stopped = await processBuzzPushGatewayMigrationGroups(
              groups: authorityGroups.values,
              initialError: resolution.firstError,
              initialStack: resolution.firstStack,
              process: (authorityTargets) async {
                final originsToQueue =
                    buzzPushGatewayMigrationGroupOriginsToQueue(
                      targets: authorityTargets,
                      replacementRelayOrigins: replacementRelayOrigins,
                    );
                if (originsToQueue.isNotEmpty) {
                  if (!attemptIsCurrent()) return true;
                  await queueBuzzPushGatewayReplacements(originsToQueue);
                  // Queueing advances the inventory generation. Let the
                  // resulting rebuild process the fully journaled group.
                  return true;
                }
                final queuedOrigins = authorityTargets
                    .map((target) => target.relayOrigin)
                    .where(replacementRelayOrigins.contains)
                    .where(
                      (origin) => !resolution.blockedOrigins.contains(origin),
                    )
                    .toSet();
                for (
                  var index = 0;
                  index < authorityTargets.length;
                  index += 1
                ) {
                  final target = authorityTargets[index];
                  await _publishCommunityReplacement(
                    ref,
                    target.community,
                    communities,
                    targetGatewayOrigin,
                    descriptor: target.descriptor,
                    forceDelegationRenewal:
                        queuedOrigins.isNotEmpty && index == 0,
                    attemptIsCurrent: attemptIsCurrent,
                  );
                }
                if (queuedOrigins.isEmpty) return false;
                if (!attemptIsCurrent()) return true;
                await checkpointBuzzPushGatewayReplacements(
                  queuedOrigins,
                  replacementGeneration,
                  token,
                );
                // The checkpoint atomically removes every origin whose grants
                // share this delegation authority. Let the resulting rebuild
                // own the next authority so attempts cannot overlap.
                return true;
              },
            );
            if (stopped) return;
            if (!attemptIsCurrent()) return;
            await completeBuzzPushGatewayMigration();
            gatewayMigrationFailures.clear(attempt);
            gatewayMigrationAttempt.complete(attempt);
          } catch (error, stack) {
            if (!attemptIsCurrent()) return;
            final failureCount = gatewayMigrationFailures.recordFailure(
              attempt,
            );
            final retryDelay = buzzPushGatewayInitializationRetryDelay(
              failureCount,
            );
            if (retryDelay == null) {
              gatewayMigrationAttempt.complete(attempt);
            } else {
              gatewayMigrationAttempt.retryAfter(
                attempt,
                delay: retryDelay,
                retry: () {
                  if (context.mounted) gatewayMigrationRetry.value += 1;
                },
              );
            }
            debugPrint('Push gateway migration failed: $error');
            if (retryDelay == null) {
              debugPrint(
                'Push gateway migration remains durably queued for the next app launch.',
              );
            }
            debugPrintStack(stackTrace: stack);
          }
        }());
        return null;
      },
      [
        token,
        migrationRelayOrigins,
        replacementRelayOrigins,
        replacementGeneration,
        communitiesAsync.hasValue,
        communities,
        gatewayMigrationRetry.value,
      ],
    );

    useEffect(
      () {
        if (!_ready(session, config, community, memberPubkey) ||
            !buzzPushLifecycleEnabled(
              community: community,
              descriptor: descriptor,
            ) ||
            token == null ||
            activeCommunityAwaitingGatewayMigration) {
          return null;
        }
        final activeCommunity = community!;
        final activeDescriptor = descriptor!;
        final state = activeCommunity.pushSubscriptionState;
        if (state.desired.isEmpty) return null;
        final attempt = buzzPushPublicationAttemptKey(
          communityId: activeCommunity.id,
          relayBaseUrl: config.baseUrl,
          token: token,
          descriptor: activeDescriptor,
          subscriptions: state.desired,
        );
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
            publicationAttempt.failed(
              attempt,
              retry: () {
                if (context.mounted) publicationRetry.value += 1;
              },
            );
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
        community?.pushSubscriptionState,
        memberPubkey,
        descriptor,
        token,
        activeCommunityAwaitingGatewayMigration,
        publicationRetry.value,
      ],
    );

    return BuzzPushRegistrationBootstrap(
      shouldRegister: activeLifecycleReady || migrationCommunities.isNotEmpty,
      attemptKey: [
        if (activeLifecycleReady) 'active:${community!.id}|${config.baseUrl}',
        if (migrationCommunities.isNotEmpty)
          'migration:${migrationCommunities.map((candidate) => candidate.id).join(',')}',
      ].join('|'),
      child: child,
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
    SignedEventRelay relay,
  ) async {
    final state = community.pushSubscriptionState;
    final desired = state.desired;
    final descriptor = await fetchBuzzPushLeaseDescriptor(config.baseUrl);
    final grant = await enrollBuzzPush(
      config.wsUrl,
      Env.pushGatewayUrl,
      communitiesForSnapshotRefresh:
          ref.read(communityListProvider).value ?? [community],
    );
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
        gatewayOrigin: buzzPushGatewayOrigin(Env.pushGatewayUrl),
      ),
    );
    return grant;
  }

  static Future<BuzzPushEndpointGrant> _publishCommunityReplacement(
    WidgetRef ref,
    Community community,
    List<Community> communities,
    String targetGatewayOrigin, {
    required BuzzPushLeaseDescriptor descriptor,
    bool forceDelegationRenewal = false,
    required bool Function() attemptIsCurrent,
  }) async {
    final config = RelayConfig(
      baseUrl: community.relayUrl,
      nsec: community.nsec,
    );
    final nsec = config.nsec;
    final memberPubkey = pubkeyFromNsec(nsec);
    if (nsec == null || nsec.isEmpty || memberPubkey == null) {
      throw StateError(
        'Cannot migrate push for ${community.id}: signing key is unavailable',
      );
    }
    final grant = await runBuzzPushGatewayMigrationMutationIfCurrent(
      attemptIsCurrent: attemptIsCurrent,
      mutate: () => enrollBuzzPush(
        config.wsUrl,
        Env.pushGatewayUrl,
        communitiesForSnapshotRefresh: communities,
        forceDelegationRenewal: forceDelegationRenewal,
      ),
    );
    final notifier = ref.read(communityListProvider.notifier);
    await publishBuzzPushLeaseRecoverably(
      reserveGeneration: () =>
          notifier.reservePushLeaseGeneration(community.id),
      operationIsCurrent: attemptIsCurrent,
      publish: (generation) => publishBuzzDevPushLease(
        grant: grant,
        leaseInstallationId: community.pushLeaseInstallationId,
        leaseGeneration: generation,
        descriptor: descriptor,
        nsec: nsec,
        memberPubkey: memberPubkey,
        subscriptions: community.pushSubscriptionState.desired,
        submit: ({required kind, required content, required tags, createdAt}) =>
            submitSignedEventOnce(
              wsUrl: config.wsUrl,
              nsec: nsec,
              kind: kind,
              content: content,
              tags: tags,
              createdAt: createdAt,
            ),
      ),
      markAccepted: (generation) =>
          markBuzzPushGatewayMigrationAcceptedIfCurrent(
            attemptIsCurrent: attemptIsCurrent,
            markAccepted: () => notifier.markPushLeaseAccepted(
              community.id,
              subscriptions: community.pushSubscriptionState.desired,
              generation: generation,
              gatewayOrigin: targetGatewayOrigin,
            ),
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
