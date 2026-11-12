import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/huddle/huddle.dart';
import '../../shared/relay/relay.dart';
import 'channel_management_provider.dart';
import 'dm_voice_agent_provider.dart';
import 'channels_provider.dart';

typedef HuddleHumanCountLoader = Future<int> Function(String channelId);

enum MobileHuddlePresentation { hidden, fullScreen, drawer }

/// Tracks whether the foreground Huddle owns the screen or lives in the
/// persistent app-level drawer. Audio lifecycle remains owned separately by
/// [huddleSessionProvider].
final class MobileHuddlePresentationNotifier
    extends Notifier<MobileHuddlePresentation> {
  @override
  MobileHuddlePresentation build() {
    ref.listen(huddleSessionProvider, (_, next) {
      if (!next.isInSession && state != MobileHuddlePresentation.hidden) {
        state = MobileHuddlePresentation.hidden;
      }
    });
    return MobileHuddlePresentation.hidden;
  }

  void showFullScreen() => state = MobileHuddlePresentation.fullScreen;

  void minimize() {
    if (ref.read(huddleSessionProvider).isInSession) {
      state = MobileHuddlePresentation.drawer;
    }
  }

  void hide() => state = MobileHuddlePresentation.hidden;
}

final mobileHuddlePresentationProvider =
    NotifierProvider<
      MobileHuddlePresentationNotifier,
      MobileHuddlePresentation
    >(MobileHuddlePresentationNotifier.new);

/// Loads the backing-channel human count used by desktop's leave rule.
///
/// A disconnected or failed lookup throws so [MobileHuddleController.leave]
/// can take desktop's safe fallback: assume another human remains and avoid
/// ending the Huddle on transient relay failure.
final huddleHumanCountProvider = Provider<HuddleHumanCountLoader>((ref) {
  return (channelId) async {
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      throw StateError('Relay is unavailable for Huddle member lookup.');
    }
    // Desktop always queries the relay's current kind:39002 snapshot here.
    // The UI-facing provider intentionally preserves cached members through
    // reconnects, but that cache can lag a leave event and must not decide
    // whether the room ends for everyone.
    final events = await ref
        .read(relaySessionProvider.notifier)
        .fetchHistory(NostrFilters.channelMembers(channelId));
    if (ref.read(relaySessionProvider).status != SessionStatus.connected) {
      throw StateError('Relay disconnected during Huddle member lookup.');
    }
    if (events.isEmpty) return 0;
    return membersFromEvent(
      events.first,
    ).where((member) => member.role != 'bot').length;
  };
});

/// Complete parent-channel Huddle lifecycle, independent of timeline paging.
final huddleLifecycleProvider = FutureProvider.family<List<NostrEvent>, String>(
  (ref, channelId) async {
    if (ref.watch(relaySessionProvider).status != SessionStatus.connected) {
      return const [];
    }
    return ref
        .read(relaySessionProvider.notifier)
        .fetchHistory(NostrFilters.huddleLifecycle(channelId));
  },
);

/// One successful relay admission. Its epoch changes only when a newer join or
/// start attempt begins, so duplicate teardown calls cannot invalidate the
/// cleanup already in flight for the same admission.
final class _HuddleAdmissionToken {
  final int epoch;
  final String backingChannelId;

  /// DM membership also grants access to the saved transcript after hanging up.
  final bool preserveMembership;

  const _HuddleAdmissionToken(
    this.epoch,
    this.backingChannelId, {
    this.preserveMembership = false,
  });
}

/// Coordinates the Nostr lifecycle around the foreground audio session.
final class MobileHuddleController extends Notifier<bool> {
  var _generation = 0;
  var _admissionEpoch = 0;
  String? _latestAdmissionTargetBackingChannelId;
  _HuddleAdmissionToken? _admissionToken;
  final Set<_HuddleAdmissionToken> _finishedLifecycleAdmissions = {};

  @override
  bool build() {
    final unregisterBeforePause = ref
        .read(relaySessionProvider.notifier)
        .registerBeforePause(_leaveForBackground);
    final unregisterBeforeCommunityTransition = ref
        .read(communityTransitionProvider)
        .register(_leaveForTransition);
    ref.onDispose(unregisterBeforePause);
    ref.onDispose(unregisterBeforeCommunityTransition);
    ref.listen(huddleSessionProvider, (previous, next) {
      if (previous?.wasAdmitted == true &&
          next.phase == HuddleSessionPhase.failed) {
        _startFailureCleanup(next);
      }
    });
    ref.listen(appLifecycleProvider, (_, next) {
      if (next == AppLifecycleState.paused ||
          next == AppLifecycleState.detached) {
        unawaited(_leaveForBackground());
      }
    });
    return false;
  }

  Future<void>? _startFuture;

  Future<void> start({required String parentChannelId, String? agentPubkey}) {
    final inFlight = _startFuture;
    if (inFlight != null) return inFlight;

    late final Future<void> startFuture;
    startFuture =
        _start(
          parentChannelId: parentChannelId,
          agentPubkey: agentPubkey,
        ).whenComplete(() {
          if (identical(_startFuture, startFuture)) _startFuture = null;
        });
    _startFuture = startFuture;
    return startFuture;
  }

  Future<void> _start({
    required String parentChannelId,
    String? agentPubkey,
  }) async {
    if (state) return;
    final generation = ++_generation;
    final admissionEpoch = ++_admissionEpoch;
    state = true;
    final actions = ref.read(channelActionsProvider);
    String? backingChannelId;
    var announced = false;
    try {
      final resolvedAgent = await resolveDmVoiceAgent(ref, parentChannelId);
      _ensureCurrent(generation);
      if (agentPubkey != null && resolvedAgent != agentPubkey) {
        throw StateError('This agent is no longer available for voice.');
      }
      agentPubkey = resolvedAgent;
      backingChannelId = await actions.createHuddleBackingChannel(
        parentChannelId: parentChannelId,
      );
      _latestAdmissionTargetBackingChannelId = backingChannelId;
      _ensureCurrent(generation);
      if (agentPubkey != null) {
        await actions.addMembers(
          channelId: backingChannelId,
          pubkeys: [agentPubkey],
          role: 'bot',
        );
        _ensureCurrent(generation);
      }
      final start = await actions.announceHuddleStarted(
        parentChannelId: parentChannelId,
        ephemeralChannelId: backingChannelId,
      );
      announced = true;
      _ensureCurrent(generation);
      final parameters = _parameters(
        parentChannelId: parentChannelId,
        ephemeralChannelId: backingChannelId,
      );
      await ref
          .read(huddleSessionProvider.notifier)
          .join(
            parameters,
            currentPubkey: ref.read(currentPubkeyProvider),
            isCreator: true,
            startedEventId: start.id,
          );
      _ensureCurrent(generation);
      final session = ref.read(huddleSessionProvider);
      if (session.phase == HuddleSessionPhase.failed) {
        throw StateError(session.error ?? 'Unable to join the new Huddle.');
      }
      _admissionToken = _HuddleAdmissionToken(
        admissionEpoch,
        backingChannelId,
        preserveMembership: _isDirectCall(parentChannelId),
      );
    } catch (_) {
      if (admissionEpoch == _admissionEpoch) {
        _latestAdmissionTargetBackingChannelId = null;
      }
      if (backingChannelId != null) {
        if (announced) {
          try {
            await actions.announceHuddleEnded(
              parentChannelId: parentChannelId,
              ephemeralChannelId: backingChannelId,
            );
          } catch (_) {
            // Continue the best-effort rollback with channel archival.
          }
        }
        try {
          await actions.archiveChannel(backingChannelId);
        } catch (_) {
          // The original start failure is more useful to the caller.
        }
      }
      rethrow;
    } finally {
      if (generation == _generation) state = false;
    }
  }

  Future<void> join({
    required String parentChannelId,
    required String ephemeralChannelId,
    required String startedBy,
    required String startedEventId,
  }) async {
    ++_generation;
    final admissionEpoch = ++_admissionEpoch;
    _latestAdmissionTargetBackingChannelId = ephemeralChannelId;
    final currentPubkey = ref.read(currentPubkeyProvider);
    final preserveMembership = _isDirectCall(parentChannelId);
    try {
      await ref
          .read(huddleSessionProvider.notifier)
          .join(
            _parameters(
              parentChannelId: parentChannelId,
              ephemeralChannelId: ephemeralChannelId,
            ),
            currentPubkey: currentPubkey,
            isCreator:
                currentPubkey != null &&
                currentPubkey.toLowerCase() == startedBy.toLowerCase(),
            startedEventId: startedEventId,
          );
    } catch (_) {
      if (admissionEpoch == _admissionEpoch) {
        _latestAdmissionTargetBackingChannelId = null;
      }
      rethrow;
    }
    final session = ref.read(huddleSessionProvider);
    if (session.wasAdmitted) {
      _admissionToken = _HuddleAdmissionToken(
        admissionEpoch,
        ephemeralChannelId,
        preserveMembership: preserveMembership,
      );
    }
  }

  Future<void>? _backgroundLeave;
  Future<void>? _failureCleanup;
  final Set<Future<void>> _lifecycleCleanups = {};

  Future<void> _leaveForBackground() {
    final inFlight = _backgroundLeave;
    if (inFlight != null) return inFlight;

    late final Future<void> backgroundLeave;
    backgroundLeave = _finishBackgroundLeave().whenComplete(() {
      if (identical(_backgroundLeave, backgroundLeave)) {
        _backgroundLeave = null;
      }
    });
    _backgroundLeave = backgroundLeave;
    return backgroundLeave;
  }

  Future<void> _finishBackgroundLeave() async {
    final generation = _lifecycleGeneration;
    Object? leaveFailure;
    StackTrace? leaveFailureStackTrace;
    try {
      await leave();
    } catch (error, stackTrace) {
      leaveFailure = error;
      leaveFailureStackTrace = stackTrace;
    }

    if (generation != _lifecycleGeneration) return;
    final failureCleanup = _failureCleanup;
    if (failureCleanup != null) await failureCleanup;
    if (generation != _lifecycleGeneration) return;
    if (_lifecycleCleanups.isNotEmpty) {
      await Future.wait(_lifecycleCleanups.toList());
    }
    if (leaveFailure != null) {
      Error.throwWithStackTrace(leaveFailure, leaveFailureStackTrace!);
    }
  }

  void _startFailureCleanup(HuddleSessionState session) {
    final cleanup = _cleanupAfterLocalTeardown(session);
    _failureCleanup = cleanup;
    unawaited(
      cleanup.whenComplete(() {
        if (identical(_failureCleanup, cleanup)) _failureCleanup = null;
      }),
    );
  }

  int _lifecycleGeneration = 0;
  Future<void>? _localLeave;

  Future<void> _leaveLocalAudio() {
    return _localLeave ??= ref
        .read(huddleSessionProvider.notifier)
        .leave()
        .whenComplete(() => _localLeave = null);
  }

  Future<void> _leaveForTransition() async {
    // Retire remote finalization before rebinding the community. In-flight
    // writes may finish in their original scope, but cannot launch another.
    ++_lifecycleGeneration;
    _backgroundLeave = null;
    _failureCleanup = null;
    _lifecycleCleanups.clear();
    _admissionToken = null;
    final startFuture = _startFuture;
    if (startFuture != null) {
      ++_generation;
      state = false;
      try {
        await startFuture;
      } catch (_) {
        // Cancellation is observed by the in-flight start, whose rollback must
        // settle before the relay is rebound to the next community.
      }
    }
    ++_generation;
    state = false;
    await _leaveLocalAudio();
  }

  Future<void> leave() async {
    final lifecycleGeneration = _lifecycleGeneration;
    ++_generation;
    state = false;
    final session = ref.read(huddleSessionProvider);
    final admissionToken = session.wasAdmitted ? _admissionToken : null;
    if (identical(_admissionToken, admissionToken)) _admissionToken = null;
    final parentChannelId = session.parentChannelId;
    final backingChannelId = session.ephemeralChannelId;
    final humanCount =
        !session.wasAdmitted ||
            backingChannelId == null ||
            _hasOtherOwnerAdmission(session)
        ? null
        : ref
              .read(huddleHumanCountProvider)(backingChannelId)
              .catchError((_) => 2);

    // Match desktop's user-facing ordering: release capture and transport
    // before waiting on relay lifecycle work. The membership query can take
    // several seconds or time out; it must never keep the microphone active
    // or hold the call page open after the user hangs up.
    Object? localFailure;
    StackTrace? localFailureStackTrace;
    try {
      await _leaveLocalAudio();
    } catch (error, stackTrace) {
      localFailure = error;
      localFailureStackTrace = stackTrace;
    }

    if (backingChannelId != null &&
        humanCount != null &&
        admissionToken != null) {
      await _trackLifecycleCleanup(
        _finishLeaveLifecycle(
          generation: lifecycleGeneration,
          admissionToken: admissionToken,
          parentChannelId: parentChannelId,
          backingChannelId: backingChannelId,
          humanCount: humanCount,
        ),
      );
    }
    if (localFailure != null) {
      Error.throwWithStackTrace(localFailure, localFailureStackTrace!);
    }
  }

  Future<void> _cleanupAfterLocalTeardown(HuddleSessionState session) async {
    final lifecycleGeneration = _lifecycleGeneration;
    final backingChannelId = session.ephemeralChannelId;
    final admissionToken = session.wasAdmitted ? _admissionToken : null;
    if (admissionToken == null || backingChannelId == null) return;
    if (_hasOtherOwnerAdmission(session)) {
      _admissionToken = null;
      return;
    }
    if (identical(_admissionToken, admissionToken)) _admissionToken = null;
    final humanCount = ref
        .read(huddleHumanCountProvider)(backingChannelId)
        .catchError((_) => 2);
    await _trackLifecycleCleanup(
      _finishLeaveLifecycle(
        generation: lifecycleGeneration,
        admissionToken: admissionToken,
        parentChannelId: session.parentChannelId,
        backingChannelId: backingChannelId,
        humanCount: humanCount,
      ),
    );
  }

  Future<void> _trackLifecycleCleanup(Future<void> cleanup) async {
    _lifecycleCleanups.add(cleanup);
    try {
      await cleanup;
    } finally {
      _lifecycleCleanups.remove(cleanup);
    }
  }

  Future<void> _finishLeaveLifecycle({
    required int generation,
    required _HuddleAdmissionToken admissionToken,
    required String? parentChannelId,
    required String backingChannelId,
    required Future<int> humanCount,
  }) async {
    if (!ref.mounted || generation != _lifecycleGeneration) return;
    final actions = ref.read(channelActionsProvider);
    final humansRemaining = await humanCount;
    if (!ref.mounted || generation != _lifecycleGeneration) return;
    if (_newerAdmissionTargetsSameHuddle(admissionToken)) return;
    final currentSession = ref.read(huddleSessionProvider);
    if (currentSession.isInSession &&
        currentSession.ephemeralChannelId == backingChannelId) {
      return;
    }
    if (!_finishedLifecycleAdmissions.add(admissionToken)) return;
    if (humansRemaining <= 1 && parentChannelId != null) {
      // Desktop auto-ends when the departing person is the last human.
      // Both lifecycle publication and archival are best effort there.
      try {
        await actions.announceHuddleEnded(
          parentChannelId: parentChannelId,
          ephemeralChannelId: backingChannelId,
        );
      } catch (_) {}
      if (!ref.mounted || generation != _lifecycleGeneration) return;
      if (_newerAdmissionTargetsSameHuddle(admissionToken)) return;
      final resumedSession = ref.read(huddleSessionProvider);
      if (resumedSession.isInSession &&
          resumedSession.ephemeralChannelId == backingChannelId) {
        return;
      }
      try {
        await actions.archiveChannel(backingChannelId);
      } catch (_) {}
      return;
    }

    if (admissionToken.preserveMembership) return;
    try {
      await actions.leaveChannel(backingChannelId);
    } catch (_) {
      // The audio relay may already have auto-ended and archived the room.
    }
  }

  bool _isDirectCall(String parentChannelId) {
    final channel = ref.read(channelByIdProvider(parentChannelId));
    return channel?.isDm == true &&
        channel!.participantPubkeys.toSet().length == 2;
  }

  bool _hasOtherOwnerAdmission(HuddleSessionState session) =>
      session.currentPubkey != null &&
      session.participantPubkeys
              .where(
                (key) =>
                    key.toLowerCase() == session.currentPubkey!.toLowerCase(),
              )
              .length >
          1;

  Future<void> end() async {
    ++_generation;
    state = false;
    final session = ref.read(huddleSessionProvider);
    final admissionToken = session.wasAdmitted ? _admissionToken : null;
    final parentChannelId = session.parentChannelId;
    final backingChannelId = session.ephemeralChannelId;
    if (!session.isCreator ||
        admissionToken == null ||
        parentChannelId == null ||
        backingChannelId == null) {
      throw StateError('Only the Huddle creator can end it.');
    }
    if (identical(_admissionToken, admissionToken)) _admissionToken = null;

    final actions = ref.read(channelActionsProvider);
    if (!_finishedLifecycleAdmissions.add(admissionToken)) return;
    Object? failure;
    try {
      await _leaveLocalAudio();
    } catch (error) {
      failure = error;
    }
    if (_newerAdmissionTargetsSameHuddle(admissionToken)) {
      if (failure != null) throw failure;
      return;
    }
    try {
      await actions.announceHuddleEnded(
        parentChannelId: parentChannelId,
        ephemeralChannelId: backingChannelId,
      );
    } catch (error) {
      failure = error;
    }
    if (_newerAdmissionTargetsSameHuddle(admissionToken)) {
      if (failure != null) throw failure;
      return;
    }
    try {
      await actions.archiveChannel(backingChannelId);
    } catch (error) {
      failure ??= error;
    }
    if (_newerAdmissionTargetsSameHuddle(admissionToken)) {
      if (failure != null) throw failure;
      return;
    }
    if (failure != null) throw failure;
  }

  bool _newerAdmissionTargetsSameHuddle(_HuddleAdmissionToken admissionToken) {
    if (admissionToken.epoch == _admissionEpoch) return false;
    return _latestAdmissionTargetBackingChannelId ==
        admissionToken.backingChannelId;
  }

  HuddleConnectionParameters _parameters({
    required String parentChannelId,
    required String ephemeralChannelId,
  }) {
    final config = ref.read(relayConfigProvider);
    final nsec = config.nsec;
    if (nsec == null || nsec.isEmpty) {
      throw StateError('A paired identity is required.');
    }
    return HuddleConnectionParameters(
      relayWebSocketUrl: config.wsUrl,
      nsec: nsec,
      parentChannelId: parentChannelId,
      ephemeralChannelId: ephemeralChannelId,
    );
  }

  void _ensureCurrent(int generation) {
    if (generation != _generation) {
      throw StateError('Huddle start was cancelled.');
    }
  }
}

/// Foreground Huddle lifecycle operations and their start-in-progress flag.
final mobileHuddleControllerProvider =
    NotifierProvider<MobileHuddleController, bool>(MobileHuddleController.new);
