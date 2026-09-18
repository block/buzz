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
    required this.workingAgentPubkeys,
    required this.retryTooltip,
    required this.retryIcon,
    required this.onRetry,
    required this.onParticipantTap,
    required this.onOverflowTap,
  });

  final bool connected;
  final String? error;
  final Map<String, UserProfile> profiles;
  final Map<String, String> fallbackLabels;
  final List<String> remotePubkeys;
  final String? localPubkey;
  final Set<String> activeSpeakerPubkeys;
  final Map<String, double> speakerLevels;
  final Set<String> workingAgentPubkeys;
  final String retryTooltip;
  final IconData retryIcon;
  final VoidCallback onRetry;
  final ValueChanged<String> onParticipantTap;
  final VoidCallback onOverflowTap;

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
        : const Duration(milliseconds: 360);
    final entryDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 420);
    final exitDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 360);

    return Stack(
      key: const ValueKey('huddle-participant-stage'),
      fit: StackFit.expand,
      children: [
        Positioned.fill(
          child: TweenAnimationBuilder<double>(
            key: const ValueKey('huddle-local-participant-motion'),
            duration: movementDuration,
            curve: Curves.easeOutBack,
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
              preparingResponse: false,
              isSelf: true,
              onTap: null,
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
              child: _HuddleParticipantCluster(
                pubkeys: remotePubkeys,
                profiles: profiles,
                fallbackLabels: fallbackLabels,
                activeSpeakerPubkeys: activeSpeakerPubkeys,
                speakerLevels: speakerLevels,
                workingAgentPubkeys: workingAgentPubkeys,
                movementDuration: movementDuration,
                entryDuration: entryDuration,
                exitDuration: exitDuration,
                onParticipantTap: onParticipantTap,
                onOverflowTap: onOverflowTap,
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
