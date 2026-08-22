part of '../channel_detail_page.dart';

class _HuddleCallParticipants extends StatelessWidget {
  const _HuddleCallParticipants({
    required this.connected,
    required this.error,
    required this.profiles,
    required this.fallbackLabels,
    required this.remotePubkeys,
    required this.localPubkey,
    required this.activeSpeakerPubkeys,
    required this.speakerLevels,
    required this.retryTooltip,
    required this.retryIcon,
    required this.onRetry,
  });

  final bool connected;
  final String? error;
  final Map<String, UserProfile> profiles;
  final Map<String, String> fallbackLabels;
  final List<String> remotePubkeys;
  final String? localPubkey;
  final Set<String> activeSpeakerPubkeys;
  final Map<String, double> speakerLevels;
  final String retryTooltip;
  final IconData retryIcon;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    if (error case final message?) {
      return Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 300),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                LucideIcons.triangleAlert,
                size: 32,
                color: context.colors.error,
              ),
              const SizedBox(height: Grid.xxs),
              Text(
                message,
                textAlign: TextAlign.center,
                style: context.textTheme.bodyMedium?.copyWith(
                  color: context.colors.error,
                ),
              ),
              const SizedBox(height: Grid.xs),
              IconButton.filledTonal(
                key: const ValueKey('huddle-retry'),
                tooltip: retryTooltip,
                onPressed: onRetry,
                icon: Icon(retryIcon),
              ),
            ],
          ),
        ),
      );
    }

    if (!connected) {
      return const Center(child: _HuddleLoadingBee());
    }

    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final hasRemoteParticipants = remotePubkeys.isNotEmpty;
    final hasDenseRemoteRoster =
        remotePubkeys.length > _huddleDenseParticipantThreshold;
    final remoteHeightFactor = hasDenseRemoteRoster ? 0.58 : 0.5;
    final localHeightFactor = hasDenseRemoteRoster ? 0.42 : 0.5;
    final movementDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 260);
    final entryDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 180);

    return Stack(
      key: const ValueKey('huddle-participant-stage'),
      fit: StackFit.expand,
      children: [
        Positioned.fill(
          child: TweenAnimationBuilder<double>(
            key: const ValueKey('huddle-local-participant-motion'),
            duration: movementDuration,
            curve: Curves.easeInOutCubic,
            tween: Tween(
              begin: hasRemoteParticipants ? 1 : 0,
              end: hasRemoteParticipants ? 1 : 0,
            ),
            builder: (context, value, child) => FractionallySizedBox(
              heightFactor: localHeightFactor,
              alignment: Alignment.lerp(
                Alignment.center,
                Alignment.bottomCenter,
                value,
              )!,
              child: Align(
                key: const ValueKey('huddle-local-participant'),
                alignment: Alignment.lerp(
                  Alignment.center,
                  const Alignment(0, -0.35),
                  value,
                )!,
                child: child,
              ),
            ),
            child: _HuddleCallAvatar(
              pubkey: localPubkey ?? '',
              profile: localPubkey == null ? null : profiles[localPubkey],
              fallbackLabel: null,
              active:
                  localPubkey != null &&
                  activeSpeakerPubkeys.contains(localPubkey),
              speakerLevel: localPubkey == null
                  ? 0
                  : speakerLevels[localPubkey] ?? 0,
              isSelf: true,
            ),
          ),
        ),
        Positioned.fill(
          child: FractionallySizedBox(
            key: const ValueKey('huddle-remote-participant-region'),
            heightFactor: remoteHeightFactor,
            alignment: Alignment.topCenter,
            child: Align(
              key: const ValueKey('huddle-remote-participant-group'),
              alignment: const Alignment(0, 0.35),
              child: LayoutBuilder(
                builder: (context, constraints) {
                  final layout = _HuddleParticipantLayout.fit(
                    availableSize: constraints.biggest,
                    participantCount: remotePubkeys.length,
                    dense: hasDenseRemoteRoster,
                  );
                  return TweenAnimationBuilder<double>(
                    key: const ValueKey('huddle-remote-participants'),
                    duration: movementDuration,
                    curve: Curves.easeInOutCubic,
                    tween: Tween(end: layout.frameSize),
                    builder: (context, frameSize, _) => Wrap(
                      alignment: WrapAlignment.center,
                      runAlignment: WrapAlignment.center,
                      spacing: layout.spacing,
                      runSpacing: layout.runSpacing,
                      children: [
                        for (final pubkey in remotePubkeys)
                          SizedBox(
                            key: ValueKey('huddle-participant-entry-$pubkey'),
                            width: frameSize,
                            height: frameSize + _huddleParticipantLabelSpace,
                            child: TweenAnimationBuilder<double>(
                              duration: entryDuration,
                              curve: Curves.easeOutCubic,
                              tween: Tween(begin: 0, end: 1),
                              builder: (context, value, child) => Opacity(
                                opacity: value,
                                child: Transform.scale(
                                  scale: 0.95 + value * 0.05,
                                  child: child,
                                ),
                              ),
                              child: _HuddleCallAvatar(
                                pubkey: pubkey,
                                profile: profiles[pubkey],
                                fallbackLabel: fallbackLabels[pubkey],
                                active: activeSpeakerPubkeys.contains(pubkey),
                                speakerLevel: speakerLevels[pubkey] ?? 0,
                                frameSize: frameSize,
                              ),
                            ),
                          ),
                      ],
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _HuddleLoadingBee extends HookWidget {
  const _HuddleLoadingBee();

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final flapController = useAnimationController(
      duration: const Duration(milliseconds: 480),
    );
    final flapProgress = useAnimation(flapController);
    useEffect(() {
      if (reducedMotion) {
        flapController
          ..stop()
          ..reset();
      } else {
        flapController.repeat();
      }
      return null;
    }, [flapController, reducedMotion]);
    final flapAmount = reducedMotion
        ? 0.0
        : 0.5 - (0.5 * cos(flapProgress * 4 * pi));

    return Semantics(
      label: 'Joining Huddle',
      liveRegion: true,
      child: ExcludeSemantics(
        child: FlappingBee(
          key: const ValueKey('huddle-loading-bee'),
          width: 60,
          color: context.colors.primary,
          flapAmount: flapAmount,
        ),
      ),
    );
  }
}

class _HuddleParticipantLayout {
  const _HuddleParticipantLayout({
    required this.frameSize,
    required this.spacing,
    required this.runSpacing,
  });

  final double frameSize;
  final double spacing;
  final double runSpacing;

  static _HuddleParticipantLayout fit({
    required Size availableSize,
    required int participantCount,
    required bool dense,
  }) {
    const spacing = Grid.xs;
    const runSpacing = Grid.xxs;
    final maximumFrameSize = dense
        ? _huddleDenseAvatarFrameSize
        : _huddleAvatarFrameSize;
    if (participantCount <= 0 || availableSize.isEmpty) {
      return _HuddleParticipantLayout(
        frameSize: maximumFrameSize,
        spacing: spacing,
        runSpacing: runSpacing,
      );
    }

    var bestFrameSize = _huddleMinimumAvatarFrameSize;
    for (var columns = 1; columns <= participantCount; columns++) {
      final rows = (participantCount + columns - 1) ~/ columns;
      final widthForAvatars = availableSize.width - spacing * (columns - 1);
      final heightForAvatars =
          availableSize.height -
          runSpacing * (rows - 1) -
          _huddleParticipantLabelSpace * rows;
      if (widthForAvatars <= 0 || heightForAvatars <= 0) continue;

      final widthBound = widthForAvatars / columns;
      final heightBound = heightForAvatars / rows;
      final candidate = min(maximumFrameSize, min(widthBound, heightBound));
      if (candidate > bestFrameSize) bestFrameSize = candidate;
    }

    return _HuddleParticipantLayout(
      frameSize: bestFrameSize.clamp(
        _huddleMinimumAvatarFrameSize,
        maximumFrameSize,
      ),
      spacing: spacing,
      runSpacing: runSpacing,
    );
  }
}
