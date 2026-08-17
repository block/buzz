import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/huddle/huddle.dart';
import '../../shared/relay/relay.dart';
import 'channel_management_provider.dart';

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

  const _HuddleAdmissionToken(this.epoch);
}

/// Coordinates the Nostr lifecycle around the foreground audio session.
final class MobileHuddleController extends Notifier<bool> {
  var _generation = 0;
  var _admissionEpoch = 0;
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
        unawaited(_cleanupAfterLocalTeardown(next));
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

  Future<void> start({required String parentChannelId}) {
    final inFlight = _startFuture;
    if (inFlight != null) return inFlight;

    late final Future<void> startFuture;
    startFuture = _start(parentChannelId: parentChannelId).whenComplete(() {
      if (identical(_startFuture, startFuture)) _startFuture = null;
    });
    _startFuture = startFuture;
    return startFuture;
  }

  Future<void> _start({required String parentChannelId}) async {
    if (state) return;
    final generation = ++_generation;
    final admissionEpoch = ++_admissionEpoch;
    state = true;
    final actions = ref.read(channelActionsProvider);
    String? backingChannelId;
    var announced = false;
    try {
      backingChannelId = await actions.createHuddleBackingChannel();
      _ensureCurrent(generation);
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
      _admissionToken = _HuddleAdmissionToken(admissionEpoch);
    } catch (_) {
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
    final currentPubkey = ref.read(currentPubkeyProvider);
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
    final session = ref.read(huddleSessionProvider);
    if (session.wasAdmitted) {
      _admissionToken = _HuddleAdmissionToken(admissionEpoch);
    }
  }

  Future<void>? _backgroundLeave;

  Future<void> _leaveForBackground() =>
      _backgroundLeave ??= leave().whenComplete(() => _backgroundLeave = null);

  Future<void> _leaveForTransition() async {
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
    if (ref.read(huddleSessionProvider).isInSession) {
      await _leaveForBackground();
    }
  }

  Future<void> leave() async {
    ++_generation;
    state = false;
    final session = ref.read(huddleSessionProvider);
    final admissionToken = session.wasAdmitted ? _admissionToken : null;
    if (identical(_admissionToken, admissionToken)) _admissionToken = null;
    final parentChannelId = session.parentChannelId;
    final backingChannelId = session.ephemeralChannelId;
    final humanCount = !session.wasAdmitted || backingChannelId == null
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
      await ref.read(huddleSessionProvider.notifier).leave();
    } catch (error, stackTrace) {
      localFailure = error;
      localFailureStackTrace = stackTrace;
    }

    if (backingChannelId != null &&
        humanCount != null &&
        admissionToken != null) {
      await _finishLeaveLifecycle(
        admissionToken: admissionToken,
        parentChannelId: parentChannelId,
        backingChannelId: backingChannelId,
        humanCount: humanCount,
      );
    }
    if (localFailure != null) {
      Error.throwWithStackTrace(localFailure, localFailureStackTrace!);
    }
  }

  Future<void> _cleanupAfterLocalTeardown(HuddleSessionState session) async {
    final backingChannelId = session.ephemeralChannelId;
    final admissionToken = session.wasAdmitted ? _admissionToken : null;
    if (admissionToken == null || backingChannelId == null) return;
    if (identical(_admissionToken, admissionToken)) _admissionToken = null;
    final humanCount = ref
        .read(huddleHumanCountProvider)(backingChannelId)
        .catchError((_) => 2);
    await _finishLeaveLifecycle(
      admissionToken: admissionToken,
      parentChannelId: session.parentChannelId,
      backingChannelId: backingChannelId,
      humanCount: humanCount,
    );
  }

  Future<void> _finishLeaveLifecycle({
    required _HuddleAdmissionToken admissionToken,
    required String? parentChannelId,
    required String backingChannelId,
    required Future<int> humanCount,
  }) async {
    if (!_finishedLifecycleAdmissions.add(admissionToken)) return;
    final actions = ref.read(channelActionsProvider);
    final humansRemaining = await humanCount;
    if (admissionToken.epoch != _admissionEpoch) return;
    if (humansRemaining <= 1 && parentChannelId != null) {
      // Desktop auto-ends when the departing person is the last human.
      // Both lifecycle publication and archival are best effort there.
      try {
        await actions.announceHuddleEnded(
          parentChannelId: parentChannelId,
          ephemeralChannelId: backingChannelId,
        );
      } catch (_) {}
      if (admissionToken.epoch != _admissionEpoch) return;
      try {
        await actions.archiveChannel(backingChannelId);
      } catch (_) {}
      return;
    }

    try {
      await actions.leaveChannel(backingChannelId);
    } catch (_) {
      // The audio relay may already have auto-ended and archived the room.
    }
  }

  Future<void> end() async {
    ++_generation;
    state = false;
    final session = ref.read(huddleSessionProvider);
    final admissionToken = session.wasAdmitted ? _admissionToken : null;
    if (identical(_admissionToken, admissionToken)) _admissionToken = null;
    final parentChannelId = session.parentChannelId;
    final backingChannelId = session.ephemeralChannelId;
    if (!session.isCreator ||
        admissionToken == null ||
        parentChannelId == null ||
        backingChannelId == null) {
      throw StateError('Only the Huddle creator can end it.');
    }

    final actions = ref.read(channelActionsProvider);
    if (!_finishedLifecycleAdmissions.add(admissionToken)) return;
    Object? failure;
    try {
      await ref.read(huddleSessionProvider.notifier).leave();
    } catch (error) {
      failure = error;
    }
    if (admissionToken.epoch != _admissionEpoch) {
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
    if (admissionToken.epoch != _admissionEpoch) {
      if (failure != null) throw failure;
      return;
    }
    try {
      await actions.archiveChannel(backingChannelId);
    } catch (error) {
      failure ??= error;
    }
    if (admissionToken.epoch != _admissionEpoch) {
      if (failure != null) throw failure;
      return;
    }
    if (failure != null) throw failure;
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
