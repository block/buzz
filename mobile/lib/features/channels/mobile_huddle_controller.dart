import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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

/// Coordinates the Nostr lifecycle around the foreground audio session.
final class MobileHuddleController extends Notifier<bool> {
  var _generation = 0;

  @override
  bool build() {
    ref.listen(appLifecycleProvider, (_, next) {
      if (next == AppLifecycleState.paused ||
          next == AppLifecycleState.detached) {
        unawaited(leave());
      }
    });
    return false;
  }

  Future<void> start({required String parentChannelId}) async {
    if (state) return;
    final generation = ++_generation;
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
  }

  Future<void> leave() async {
    ++_generation;
    state = false;
    final session = ref.read(huddleSessionProvider);
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

    if (backingChannelId != null && humanCount != null) {
      await _finishLeaveLifecycle(
        parentChannelId: parentChannelId,
        backingChannelId: backingChannelId,
        humanCount: humanCount,
      );
    }
    if (localFailure != null) {
      Error.throwWithStackTrace(localFailure, localFailureStackTrace!);
    }
  }

  Future<void> _finishLeaveLifecycle({
    required String? parentChannelId,
    required String backingChannelId,
    required Future<int> humanCount,
  }) async {
    final actions = ref.read(channelActionsProvider);
    final humansRemaining = await humanCount;
    if (humansRemaining <= 1 && parentChannelId != null) {
      // Desktop auto-ends when the departing person is the last human.
      // Both lifecycle publication and archival are best effort there.
      try {
        await actions.announceHuddleEnded(
          parentChannelId: parentChannelId,
          ephemeralChannelId: backingChannelId,
        );
      } catch (_) {}
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
    final parentChannelId = session.parentChannelId;
    final backingChannelId = session.ephemeralChannelId;
    if (!session.isCreator ||
        parentChannelId == null ||
        backingChannelId == null) {
      throw StateError('Only the Huddle creator can end it.');
    }

    final actions = ref.read(channelActionsProvider);
    Object? failure;
    try {
      await actions.announceHuddleEnded(
        parentChannelId: parentChannelId,
        ephemeralChannelId: backingChannelId,
      );
    } catch (error) {
      failure = error;
    }
    try {
      await actions.archiveChannel(backingChannelId);
    } catch (error) {
      failure ??= error;
    }
    await ref.read(huddleSessionProvider.notifier).leave();
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
