part of '../channel_detail_page.dart';

const _huddleLifetime = Duration(hours: 1);
const _huddleAvatarRadius = 52.0;
const _huddleAvatarFrameSize = 128.0;
const _huddleSpeakingRingSize = 112.0;
const _huddleDenseAvatarFrameSize = 104.0;
const _huddleMinimumAvatarFrameSize = 48.0;
const _huddleParticipantLabelSpace = 28.0;
const _huddleDenseParticipantThreshold = 6;

final _huddleParticipantProfileUpdatesProvider = NotifierProvider.autoDispose
    .family<_HuddleParticipantProfileUpdates, int, String>(
      _HuddleParticipantProfileUpdates.new,
    );

class _HuddleParticipantProfileUpdates extends Notifier<int> {
  _HuddleParticipantProfileUpdates(this.channelId);

  final String channelId;
  void Function()? _unsubscribe;
  int _subscriptionVersion = 0;

  @override
  int build() {
    final session = ref.watch(
      huddleSessionProvider.select(
        (state) => (
          ephemeralChannelId: state.ephemeralChannelId,
          currentPubkey: state.currentPubkey,
          participantPubkeys: state.participantPubkeys,
          wasAdmitted: state.wasAdmitted,
        ),
      ),
    );
    final members = session.wasAdmitted
        ? const <ChannelMember>[]
        : ref.watch(channelMembersProvider(channelId)).value ??
              const <ChannelMember>[];
    final relayState = ref.watch(relaySessionProvider);
    final participantPubkeys = _huddleParticipantPubkeys(
      sessionParticipantPubkeys: session.ephemeralChannelId == channelId
          ? session.participantPubkeys
          : const [],
      currentPubkey: session.currentPubkey,
      members: members,
    );
    final subscriptionVersion = ++_subscriptionVersion;
    _clearSubscription();
    ref.onDispose(() {
      _subscriptionVersion++;
      _clearSubscription();
    });

    if (participantPubkeys.isEmpty) return 0;
    ref.read(userCacheProvider.notifier).preload(participantPubkeys);
    if (relayState.status == SessionStatus.connected) {
      Future.microtask(
        () => _subscribe(participantPubkeys, subscriptionVersion),
      );
    }
    return 0;
  }

  Future<void> _subscribe(
    List<String> participantPubkeys,
    int subscriptionVersion,
  ) async {
    try {
      final unsubscribe = await ref
          .read(relaySessionProvider.notifier)
          .subscribe(
            NostrFilter(
              kinds: const [0],
              authors: participantPubkeys,
              limit: 0,
            ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000 - 5),
            (event) {
              if (!_isCurrent(subscriptionVersion)) return;
              ref.read(userCacheProvider.notifier).cacheProfileEvent(event);
              state++;
            },
          );
      if (!_isCurrent(subscriptionVersion)) {
        unsubscribe();
        return;
      }
      _unsubscribe = unsubscribe;
    } catch (error) {
      if (_isCurrent(subscriptionVersion)) {
        debugPrint('[HuddleProfiles] live profile subscription failed: $error');
      }
    }
  }

  bool _isCurrent(int subscriptionVersion) =>
      subscriptionVersion == _subscriptionVersion;

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}

List<String> _huddleParticipantPubkeys({
  required Iterable<String> sessionParticipantPubkeys,
  required String? currentPubkey,
  required Iterable<ChannelMember> members,
}) {
  final localPubkey = currentPubkey?.toLowerCase();
  final pubkeys = <String>{};
  for (final pubkey in sessionParticipantPubkeys) {
    final normalized = pubkey.trim().toLowerCase();
    if (normalized.isNotEmpty && normalized != localPubkey) {
      pubkeys.add(normalized);
    }
  }
  for (final member in members) {
    final normalized = member.pubkey.trim().toLowerCase();
    if (normalized.isNotEmpty && normalized != localPubkey) {
      pubkeys.add(normalized);
    }
  }
  if (localPubkey != null && localPubkey.isNotEmpty) {
    pubkeys.add(localPubkey);
  }
  return List.unmodifiable(pubkeys);
}

class _HuddleInvite {
  final String parentChannelId;
  final String ephemeralChannelId;
  final String startedBy;
  final String startedEventId;

  const _HuddleInvite({
    required this.parentChannelId,
    required this.ephemeralChannelId,
    required this.startedBy,
    required this.startedEventId,
  });
}

class _HuddleButton extends ConsumerWidget {
  const _HuddleButton({required this.channel, required this.events});

  final Channel channel;
  final List<NostrEvent> events;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final session = ref.watch(huddleSessionProvider);
    final latestStart = _activeHuddleStart(events);
    final unavailableFailure =
        session.phase == HuddleSessionPhase.failed &&
        _isUnavailableHuddleError(session.error);
    final activeStart =
        unavailableFailure &&
            session.startedEventId == latestStart?.startedEventId
        ? null
        : latestStart;
    final isCurrentParent =
        session.parentChannelId == channel.id &&
        session.ephemeralChannelId != null &&
        session.phase != HuddleSessionPhase.idle &&
        !unavailableFailure;
    final disabledByOtherHuddle =
        session.isInSession && session.parentChannelId != channel.id;

    return IconButton(
      key: const ValueKey('channel-huddle-button'),
      color: context.colors.primary,
      onPressed: disabledByOtherHuddle
          ? null
          : () async {
              if (isCurrentParent) {
                _showMobileHuddleCall(
                  context: context,
                  ref: ref,
                  invite: _HuddleInvite(
                    parentChannelId: channel.id,
                    ephemeralChannelId: session.ephemeralChannelId!,
                    startedBy: session.isCreator
                        ? session.currentPubkey ?? ''
                        : '',
                    startedEventId: session.startedEventId ?? '',
                  ),
                );
                return;
              }
              if (activeStart != null) {
                _openMobileHuddle(
                  context: context,
                  ref: ref,
                  invite: activeStart,
                );
                return;
              }
              try {
                await ref
                    .read(mobileHuddleControllerProvider.notifier)
                    .start(parentChannelId: channel.id);
                if (!context.mounted) return;
                final started = ref.read(huddleSessionProvider);
                final ephemeralChannelId = started.ephemeralChannelId;
                if (ephemeralChannelId == null) return;
                _showMobileHuddleCall(
                  context: context,
                  ref: ref,
                  invite: _HuddleInvite(
                    parentChannelId: channel.id,
                    ephemeralChannelId: ephemeralChannelId,
                    startedBy: started.currentPubkey ?? '',
                    startedEventId: started.startedEventId ?? '',
                  ),
                );
              } catch (error) {
                if (!context.mounted) return;
                ScaffoldMessenger.of(context).showSnackBar(
                  SnackBar(content: Text(_huddleActionError(error))),
                );
              }
            },
      tooltip: disabledByOtherHuddle
          ? 'Leave your current Huddle first'
          : activeStart != null || isCurrentParent
          ? 'Open Huddle'
          : 'Start Huddle',
      icon: const Icon(LucideIcons.headphones, size: 22),
    );
  }
}

class _HuddleJoinSurface extends ConsumerWidget {
  const _HuddleJoinSurface({
    required this.message,
    required this.allMessages,
    required this.parentChannelId,
    required this.isMember,
    required this.isArchived,
  });

  final TimelineMessage message;
  final List<TimelineMessage> allMessages;
  final String parentChannelId;
  final bool isMember;
  final bool isArchived;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ephemeralChannelId = message.systemEvent?.ephemeralChannelId;
    if (ephemeralChannelId == null) return const SizedBox.shrink();
    final session = ref.watch(huddleSessionProvider);
    final isThisSession =
        session.ephemeralChannelId == ephemeralChannelId &&
        session.phase != HuddleSessionPhase.idle;
    final unavailable =
        isThisSession &&
        session.phase == HuddleSessionPhase.failed &&
        _isUnavailableHuddleError(session.error);
    final ended = _huddleEnded(
      start: message,
      allMessages: allMessages,
      ephemeralChannelId: ephemeralChannelId,
    );
    final stale =
        DateTime.now().difference(
          DateTime.fromMillisecondsSinceEpoch(message.createdAt * 1000),
        ) >
        _huddleLifetime;
    final blockedByOtherHuddle = session.isInSession && !isThisSession;
    final canJoin =
        isMember && !isArchived && !ended && !stale && !blockedByOtherHuddle;

    final label = unavailable
        ? 'Start new'
        : switch ((isThisSession, session.phase, ended || stale)) {
            (true, HuddleSessionPhase.connected, _) => 'Open',
            (true, HuddleSessionPhase.interrupted, _) => 'Open',
            (true, HuddleSessionPhase.reconnecting, _) => 'Reconnecting',
            (true, HuddleSessionPhase.failed, _) => 'Retry',
            (true, _, _) => 'Joining',
            (_, _, true) => 'Ended',
            _ => 'Join',
          };

    return Padding(
      padding: const EdgeInsets.only(
        left: messageAvatarSize + messageAvatarContentGap,
        top: Grid.xxs,
      ),
      child: Container(
        padding: const EdgeInsets.all(Grid.twelve),
        decoration: BoxDecoration(
          color: context.colors.primary.withValues(alpha: 0.08),
          borderRadius: BorderRadius.circular(Radii.md),
          border: Border.all(
            color: context.colors.primary.withValues(alpha: 0.18),
          ),
        ),
        child: Row(
          children: [
            Icon(
              LucideIcons.headphones,
              size: 20,
              color: context.colors.primary,
            ),
            const SizedBox(width: Grid.xxs),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    ended || stale || unavailable
                        ? 'Huddle ended'
                        : 'Huddle in progress',
                    style: context.textTheme.labelLarge?.copyWith(
                      color: context.colors.onSurface,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  Text(
                    isThisSession && session.isConnected
                        ? '${session.participantCount} connected'
                        : 'Foreground voice Huddle',
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            FilledButton.tonal(
              key: ValueKey('huddle-$label-$ephemeralChannelId'),
              onPressed:
                  (canJoin || isThisSession) &&
                      session.phase != HuddleSessionPhase.leaving
                  ? () => unavailable
                        ? unawaited(
                            _startMobileHuddleFromChannel(
                              context: context,
                              ref: ref,
                              parentChannelId: parentChannelId,
                            ),
                          )
                        : _openMobileHuddle(
                            context: context,
                            ref: ref,
                            invite: _HuddleInvite(
                              parentChannelId: parentChannelId,
                              ephemeralChannelId: ephemeralChannelId,
                              startedBy: message.pubkey,
                              startedEventId: message.id,
                            ),
                          )
                  : null,
              child: Text(label),
            ),
          ],
        ),
      ),
    );
  }
}

bool _huddleEnded({
  required TimelineMessage start,
  required List<TimelineMessage> allMessages,
  required String ephemeralChannelId,
}) => allMessages.any((candidate) {
  final event = candidate.systemEvent;
  return candidate.createdAt >= start.createdAt &&
      event?.type == SystemEventType.huddleEnded &&
      event?.ephemeralChannelId == ephemeralChannelId;
});

Future<void> _startMobileHuddleFromChannel({
  required BuildContext context,
  required WidgetRef ref,
  required String parentChannelId,
}) async {
  try {
    await ref
        .read(mobileHuddleControllerProvider.notifier)
        .start(parentChannelId: parentChannelId);
    if (!context.mounted) return;
    final session = ref.read(huddleSessionProvider);
    final ephemeralChannelId = session.ephemeralChannelId;
    if (ephemeralChannelId == null ||
        session.phase == HuddleSessionPhase.failed) {
      throw StateError(session.error ?? 'Unable to start a new Huddle.');
    }
    _showMobileHuddleCall(
      context: context,
      ref: ref,
      invite: _HuddleInvite(
        parentChannelId: parentChannelId,
        ephemeralChannelId: ephemeralChannelId,
        startedBy: session.currentPubkey ?? '',
        startedEventId: session.startedEventId ?? '',
      ),
    );
  } catch (error) {
    if (!context.mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(_huddleActionError(error))));
  }
}

void _openMobileHuddle({
  required BuildContext context,
  required WidgetRef ref,
  required _HuddleInvite invite,
}) {
  final config = ref.read(relayConfigProvider);
  final nsec = config.nsec;
  if (nsec == null || nsec.isEmpty) {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('A paired identity is required.')),
    );
    return;
  }

  final session = ref.read(huddleSessionProvider);
  _showMobileHuddleCall(context: context, ref: ref, invite: invite);
  if (session.ephemeralChannelId != invite.ephemeralChannelId ||
      session.phase == HuddleSessionPhase.idle ||
      session.phase == HuddleSessionPhase.failed) {
    unawaited(
      ref
          .read(mobileHuddleControllerProvider.notifier)
          .join(
            parentChannelId: invite.parentChannelId,
            ephemeralChannelId: invite.ephemeralChannelId,
            startedBy: invite.startedBy,
            startedEventId: invite.startedEventId,
          ),
    );
  }
}

void _showMobileHuddleCall({
  required BuildContext context,
  required WidgetRef ref,
  required _HuddleInvite invite,
}) {
  ref.read(mobileHuddlePresentationProvider.notifier).showFullScreen();
  unawaited(
    Navigator.of(
      context,
      rootNavigator: true,
    ).push<void>(_mobileHuddleCallRoute(context: context, invite: invite)),
  );
}

PageRouteBuilder<void> _mobileHuddleCallRoute({
  required BuildContext context,
  required _HuddleInvite invite,
}) {
  final reduceMotion = MediaQuery.disableAnimationsOf(context);
  return PageRouteBuilder<void>(
    opaque: true,
    transitionDuration: reduceMotion
        ? Duration.zero
        : const Duration(milliseconds: 280),
    reverseTransitionDuration: reduceMotion
        ? Duration.zero
        : const Duration(milliseconds: 220),
    pageBuilder: (_, _, _) => _MobileHuddleCallPage(invite: invite),
    transitionsBuilder: (_, animation, _, child) {
      if (reduceMotion) return child;
      return SlideTransition(
        position: Tween<Offset>(begin: const Offset(0, 1), end: Offset.zero)
            .animate(
              CurvedAnimation(
                parent: animation,
                curve: Curves.easeOutCubic,
                reverseCurve: Curves.easeInCubic,
              ),
            ),
        child: child,
      );
    },
  );
}

Future<void> _startReplacementHuddle({
  required BuildContext context,
  required WidgetRef ref,
  required String parentChannelId,
}) async {
  try {
    await ref
        .read(mobileHuddleControllerProvider.notifier)
        .start(parentChannelId: parentChannelId);
    if (!context.mounted) return;
    final session = ref.read(huddleSessionProvider);
    final ephemeralChannelId = session.ephemeralChannelId;
    if (ephemeralChannelId == null ||
        session.phase == HuddleSessionPhase.failed) {
      throw StateError(session.error ?? 'Unable to start a new Huddle.');
    }
    ref.read(mobileHuddlePresentationProvider.notifier).showFullScreen();
    Navigator.of(context, rootNavigator: true).pushReplacement<void, void>(
      _mobileHuddleCallRoute(
        context: context,
        invite: _HuddleInvite(
          parentChannelId: parentChannelId,
          ephemeralChannelId: ephemeralChannelId,
          startedBy: session.currentPubkey ?? '',
          startedEventId: session.startedEventId ?? '',
        ),
      ),
    );
  } catch (error) {
    if (!context.mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(_huddleActionError(error))));
  }
}

class _MobileHuddleCallPage extends ConsumerWidget {
  const _MobileHuddleCallPage({required this.invite});

  final _HuddleInvite invite;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final session = ref.watch(huddleSessionProvider);
    final sessionController = ref.read(huddleSessionProvider.notifier);
    final lifecycleController = ref.read(
      mobileHuddleControllerProvider.notifier,
    );
    final presentationController = ref.read(
      mobileHuddlePresentationProvider.notifier,
    );

    final connected =
        session.phase == HuddleSessionPhase.connected ||
        session.phase == HuddleSessionPhase.interrupted;
    final unavailable =
        session.phase == HuddleSessionPhase.failed &&
        _isUnavailableHuddleError(session.error);
    final needsMicrophoneSettings =
        session.phase == HuddleSessionPhase.failed &&
        !unavailable &&
        session.microphonePermissionRequired;
    final localPubkey = session.currentPubkey?.toLowerCase();
    final backingMembers =
        ref.watch(channelMembersProvider(invite.ephemeralChannelId)).value ??
        const <ChannelMember>[];
    final participantPubkeys = _huddleParticipantPubkeys(
      sessionParticipantPubkeys: session.participantPubkeys,
      currentPubkey: localPubkey,
      members: session.wasAdmitted ? const [] : backingMembers,
    );
    final remotePubkeys = participantPubkeys
        .where((pubkey) => pubkey != localPubkey)
        .toList(growable: false);
    ref.read(userCacheProvider.notifier).preload(participantPubkeys);
    ref.watch(
      _huddleParticipantProfileUpdatesProvider(invite.ephemeralChannelId),
    );
    final profiles = ref.watch(userCacheProvider);
    final directoryDisplayNames = ref.watch(agentDirectoryDisplayNamesProvider);
    final reactionSenderName = _huddleReactionSenderName(
      localPubkey: localPubkey,
      profile: localPubkey == null ? null : profiles[localPubkey],
    );

    final (retryTooltip, retryIcon, onRetry) = switch (true) {
      _ when unavailable => (
        'Start a new Huddle',
        LucideIcons.headphones,
        () => unawaited(
          _startReplacementHuddle(
            context: context,
            ref: ref,
            parentChannelId: invite.parentChannelId,
          ),
        ),
      ),
      _ when needsMicrophoneSettings => (
        'Open Settings',
        LucideIcons.settings,
        () => unawaited(sessionController.openMicrophoneSettings()),
      ),
      _ => (
        'Try again',
        LucideIcons.refreshCw,
        () => unawaited(
          lifecycleController.join(
            parentChannelId: invite.parentChannelId,
            ephemeralChannelId: invite.ephemeralChannelId,
            startedBy: invite.startedBy,
            startedEventId: invite.startedEventId,
          ),
        ),
      ),
    };

    return PopScope<void>(
      onPopInvokedWithResult: (didPop, _) {
        if (didPop &&
            ref.read(mobileHuddlePresentationProvider) ==
                MobileHuddlePresentation.fullScreen) {
          presentationController.minimize();
        }
      },
      child: Scaffold(
        backgroundColor: context.colors.surface,
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              Grid.xxs,
              Grid.gutter,
              Grid.gutter,
            ),
            child: Column(
              children: [
                _HuddleCallHeader(
                  isLeaving: session.phase == HuddleSessionPhase.leaving,
                  onMinimize: () {
                    presentationController.minimize();
                    Navigator.of(context).pop();
                  },
                  onLeave: () {
                    presentationController.hide();
                    final leaving = lifecycleController.leave();
                    Navigator.of(context).pop();
                    unawaited(
                      leaving.catchError((Object error) {
                        debugPrint('[MobileHuddle] leave failed: $error');
                      }),
                    );
                  },
                ),
                Expanded(
                  child: _HuddleCallParticipants(
                    connected: connected,
                    error: session.phase == HuddleSessionPhase.failed
                        ? unavailable
                              ? 'This Huddle is no longer available.'
                              : session.error ?? 'Huddle audio failed.'
                        : null,
                    profiles: profiles,
                    fallbackLabels: directoryDisplayNames,
                    remotePubkeys: remotePubkeys,
                    localPubkey: localPubkey,
                    activeSpeakerPubkeys: session.activeSpeakerPubkeys,
                    speakerLevels: session.speakerLevels,
                    retryTooltip: retryTooltip,
                    retryIcon: retryIcon,
                    onRetry: onRetry,
                  ),
                ),
                if (connected)
                  _HuddleCallControls(
                    isMuted: session.isMuted,
                    isSpeakerEnabled: session.isSpeakerEnabled,
                    onToggleMute: () =>
                        unawaited(sessionController.setMuted(!session.isMuted)),
                    onToggleSpeaker: () => unawaited(
                      sessionController.setSpeakerEnabled(
                        !session.isSpeakerEnabled,
                      ),
                    ),
                    onReact: () => showEmojiPicker(
                      context: context,
                      onSelect: (emoji) {
                        ref.read(recentEmojiProvider.notifier).record(emoji);
                        _burstHuddleReaction(
                          ref: ref,
                          fallbackContext: context,
                          emoji: emoji,
                          senderPubkey: localPubkey,
                        );
                        unawaited(
                          ref
                              .read(channelActionsProvider)
                              .sendHuddleReaction(
                                channelId: invite.ephemeralChannelId,
                                emoji: emoji,
                                senderName: reactionSenderName,
                              )
                              .catchError((Object error) {
                                if (!context.mounted) return;
                                ScaffoldMessenger.of(context).showSnackBar(
                                  const SnackBar(
                                    content: Text('Reaction failed'),
                                  ),
                                );
                              }),
                        );
                      },
                    ),
                  )
                else
                  const SizedBox(height: Grid.xxl),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

String _huddleReactionSenderName({
  required String? localPubkey,
  required UserProfile? profile,
}) {
  final displayName = profile?.displayName?.trim();
  if (displayName?.isNotEmpty == true) return displayName!;
  if (localPubkey?.isNotEmpty == true) {
    return 'Participant ${shortPubkey(localPubkey!)}';
  }
  return 'Someone';
}

class _HuddleCallHeader extends StatelessWidget {
  const _HuddleCallHeader({
    required this.isLeaving,
    required this.onMinimize,
    required this.onLeave,
  });

  final bool isLeaving;
  final VoidCallback onMinimize;
  final VoidCallback onLeave;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 72,
      child: Row(
        children: [
          IconButton(
            key: const ValueKey('huddle-minimize'),
            tooltip: 'Minimize',
            onPressed: onMinimize,
            icon: const Icon(LucideIcons.chevronDown, size: 32),
          ),
          const Spacer(),
          _HuddleRoundControl(
            key: const ValueKey('huddle-leave'),
            tooltip: 'Leave Huddle',
            icon: LucideIcons.phoneOff,
            foregroundColor: context.colors.error,
            backgroundColor: context.colors.surfaceContainerHighest,
            onPressed: isLeaving ? null : onLeave,
          ),
        ],
      ),
    );
  }
}

_HuddleInvite? _activeHuddleStart(List<NostrEvent> events) {
  final endedAt = <String, int>{};
  for (final event in events) {
    if (event.kind != EventKind.huddleEnded) continue;
    final parsed = SystemEvent.fromHuddleEvent(event);
    final id = parsed?.ephemeralChannelId;
    if (id != null && event.createdAt > (endedAt[id] ?? 0)) {
      endedAt[id] = event.createdAt;
    }
  }
  final starts =
      events.where((event) => event.kind == EventKind.huddleStarted).toList()
        ..sort((left, right) => right.createdAt.compareTo(left.createdAt));
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  for (final event in starts) {
    if (now - event.createdAt > _huddleLifetime.inSeconds) continue;
    final parsed = SystemEvent.fromHuddleEvent(event);
    final ephemeralChannelId = parsed?.ephemeralChannelId;
    final parentChannelId = event.channelId;
    if (ephemeralChannelId == null || parentChannelId == null) continue;
    if ((endedAt[ephemeralChannelId] ?? 0) >= event.createdAt) continue;
    return _HuddleInvite(
      parentChannelId: parentChannelId,
      ephemeralChannelId: ephemeralChannelId,
      startedBy: event.pubkey,
      startedEventId: event.id,
    );
  }
  return null;
}

String _huddleActionError(Object error) {
  final message = error.toString();
  return message
      .replaceFirst('Bad state: ', '')
      .replaceFirst('Invalid argument(s): ', '');
}

bool _isUnavailableHuddleError(String? error) =>
    error?.trim().toLowerCase() == 'not a member';
