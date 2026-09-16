part of '../channel_detail_page.dart';

const _huddleAgentResponseResetDelay = Duration(milliseconds: 1200);

class _HuddleCallAvatar extends HookConsumerWidget {
  const _HuddleCallAvatar({
    required this.pubkey,
    required this.profile,
    required this.fallbackLabel,
    required this.active,
    required this.speakerLevel,
    required this.preparingResponse,
    required this.onTap,
    this.isSelf = false,
    this.frameSize = _huddleAvatarFrameSize,
  });

  final String pubkey;
  final UserProfile? profile;
  final String? fallbackLabel;
  final bool active;
  final double speakerLevel;
  final bool preparingResponse;
  final VoidCallback? onTap;
  final bool isSelf;
  final double frameSize;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final responseHasStarted = useState(false);
    final responseResetTimer = useRef<Timer?>(null);
    // Tracks the previous working signal so we can detect its edges, and
    // whether the working signal has cleared since audio latched the response.
    // A completed working cycle distinguishes a genuinely new turn from late
    // typing that merely trails the turn already spoken.
    final wasPreparing = useRef(false);
    final workingCycleCompleted = useRef(false);
    useEffect(() {
      final preparingStarted = preparingResponse && !wasPreparing.value;
      final preparingCleared = !preparingResponse && wasPreparing.value;
      wasPreparing.value = preparingResponse;
      if (active) {
        // Audio for the current turn latches that a response has begun and
        // starts a fresh turn, so any prior working cycle no longer applies.
        responseResetTimer.value?.cancel();
        responseResetTimer.value = null;
        responseHasStarted.value = true;
        workingCycleCompleted.value = false;
      } else if (preparingStarted &&
          responseHasStarted.value &&
          workingCycleCompleted.value) {
        // A new working turn began after the previous turn's working signal
        // already cleared, so the "already spoke" suppression no longer
        // applies — allow the preparing indicator to show again.
        responseResetTimer.value?.cancel();
        responseResetTimer.value = null;
        responseHasStarted.value = false;
        workingCycleCompleted.value = false;
      } else if (preparingResponse) {
        // Working signal (including late typing for the turn just spoken) holds
        // the suppression alive; keep the reset timer cancelled.
        responseResetTimer.value?.cancel();
        responseResetTimer.value = null;
      } else if (responseHasStarted.value && responseResetTimer.value == null) {
        responseResetTimer.value = Timer(_huddleAgentResponseResetDelay, () {
          responseResetTimer.value = null;
          responseHasStarted.value = false;
          workingCycleCompleted.value = false;
        });
      }
      // Record that this turn's working signal has completed a cycle once it
      // clears after audio latched, so the next working turn is not mistaken
      // for trailing typing.
      if (preparingCleared && responseHasStarted.value) {
        workingCycleCompleted.value = true;
      }
      return null;
    }, [active, preparingResponse, responseHasStarted.value]);
    useEffect(
      () =>
          () => responseResetTimer.value?.cancel(),
      const [],
    );
    final showPreparingResponse =
        preparingResponse && !active && !responseHasStarted.value;
    final scale = frameSize / _huddleAvatarFrameSize;
    final avatarRadius = _huddleAvatarRadius * scale;
    final speakingRingSize = _huddleSpeakingRingSize * scale;
    final fallbackIconSize = 44 * scale;
    final normalizedSpeakerLevel = speakerLevel.clamp(0.0, 1.0).toDouble();
    final haloLevel = active ? 0.08 + normalizedSpeakerLevel * 0.92 : 0.0;
    final haloController = useAnimationController(
      initialValue: reducedMotion ? haloLevel : 0,
    );
    final animatedHaloLevel = useAnimation(haloController);
    useEffect(() {
      haloController.stop();
      if (reducedMotion) {
        haloController.value = haloLevel;
        return null;
      }

      final isRising = haloLevel > haloController.value;
      haloController.animateTo(
        haloLevel,
        duration: Duration(milliseconds: isRising ? 140 : 220),
        curve: isRising ? Curves.easeOutCubic : Curves.easeInOutCubic,
      );
      return null;
    }, [haloController, haloLevel, reducedMotion]);
    final originKey = useMemoized(
      () => GlobalKey(debugLabel: 'huddle-reaction-origin-$pubkey'),
      [pubkey],
    );
    final originRegistry = ref.read(_huddleAvatarOriginRegistryProvider);
    useEffect(() {
      originRegistry.register(pubkey, originKey);
      return () => originRegistry.unregister(pubkey, originKey);
    }, [originRegistry, pubkey, originKey]);
    final label = _huddleParticipantLabel(
      pubkey: pubkey,
      profile: profile,
      fallbackLabel: fallbackLabel,
      isSelf: isSelf,
    );
    final isAgent = profile?.isAgent == true || fallbackLabel != null;

    final semanticStates = [
      label,
      if (showPreparingResponse) 'preparing a response',
      if (active) 'speaking',
    ].join(', ');

    return SizedBox(
      width: frameSize,
      child: Semantics(
        label: semanticStates,
        hint: onTap == null ? null : 'Tap to focus participant',
        button: onTap != null,
        // The outer node excludes descendant semantics, so the child
        // indicator's live region never reaches assistive tech. Promote this
        // node to a live region while preparing so the label change announces.
        liveRegion: showPreparingResponse,
        onTap: onTap,
        excludeSemantics: true,
        child: GestureDetector(
          key: ValueKey('huddle-participant-avatar-$pubkey'),
          behavior: onTap == null
              ? HitTestBehavior.deferToChild
              : HitTestBehavior.opaque,
          excludeFromSemantics: true,
          onTap: onTap,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              SizedBox.square(
                key: ValueKey('huddle-speaking-ring-$pubkey'),
                dimension: frameSize,
                child: SizedBox.square(
                  key: originKey,
                  dimension: frameSize,
                  child: Stack(
                    alignment: Alignment.center,
                    clipBehavior: Clip.none,
                    children: [
                      AnimatedOpacity(
                        key: ValueKey('huddle-speaking-halo-opacity-$pubkey'),
                        duration: reducedMotion
                            ? Duration.zero
                            : Duration(milliseconds: active ? 140 : 220),
                        curve: active
                            ? Curves.easeOutCubic
                            : Curves.easeInOutCubic,
                        opacity: active ? 1 : 0,
                        child: Transform.scale(
                          key: ValueKey('huddle-speaking-halo-scale-$pubkey'),
                          scale: reducedMotion
                              ? (active ? 1.15 : 1)
                              : 1 + animatedHaloLevel * 1.55,
                          child: Container(
                            key: ValueKey('huddle-speaking-halo-$pubkey'),
                            width: speakingRingSize,
                            height: speakingRingSize,
                            decoration: BoxDecoration(
                              shape: isAgent
                                  ? BoxShape.rectangle
                                  : BoxShape.circle,
                              borderRadius: isAgent
                                  ? BorderRadius.circular(
                                      speakingRingSize * 0.3,
                                    )
                                  : null,
                              color: context.colors.primary.withValues(
                                alpha: 0.07,
                              ),
                            ),
                          ),
                        ),
                      ),
                      AnimatedSwitcher(
                        duration: reducedMotion
                            ? Duration.zero
                            : const Duration(milliseconds: 180),
                        switchInCurve: Curves.easeOutCubic,
                        switchOutCurve: Curves.easeInCubic,
                        transitionBuilder: (child, animation) =>
                            FadeTransition(opacity: animation, child: child),
                        child: showPreparingResponse
                            ? Container(
                                key: ValueKey(
                                  'huddle-agent-preparing-response-$pubkey',
                                ),
                                width: avatarRadius * 2,
                                height: avatarRadius * 2,
                                decoration: BoxDecoration(
                                  shape: isAgent
                                      ? BoxShape.rectangle
                                      : BoxShape.circle,
                                  borderRadius: isAgent
                                      ? BorderRadius.circular(
                                          avatarRadius * 0.6,
                                        )
                                      : null,
                                  color: context.colors.primaryContainer,
                                ),
                                alignment: Alignment.center,
                                child: BouncingDotsIndicator(
                                  color: context.colors.onPrimaryContainer,
                                  dotSize: 6 * scale,
                                  gap: 4 * scale,
                                  semanticLabel:
                                      '$label is preparing a response',
                                ),
                              )
                            : AvatarImage(
                                key: ValueKey('huddle-avatar-image-$pubkey'),
                                imageUrl: profile?.avatarUrl,
                                radius: avatarRadius,
                                backgroundColor:
                                    context.colors.primaryContainer,
                                fallback: Icon(
                                  LucideIcons.userRound,
                                  size: fallbackIconSize,
                                  color: context.colors.onPrimaryContainer,
                                ),
                                isAgent: isAgent,
                              ),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

String _huddleParticipantLabel({
  required String pubkey,
  required UserProfile? profile,
  required String? fallbackLabel,
  required bool isSelf,
}) {
  if (isSelf) return 'You';
  final profileName = profile?.displayName?.trim();
  final directoryName = fallbackLabel?.trim();
  return (profileName?.isNotEmpty == true ? profileName : null) ??
      (directoryName?.isNotEmpty == true ? directoryName : null) ??
      (pubkey.isEmpty ? 'Participant' : shortPubkey(pubkey));
}
