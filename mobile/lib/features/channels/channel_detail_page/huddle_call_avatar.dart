part of '../channel_detail_page.dart';

class _HuddleCallAvatar extends HookConsumerWidget {
  const _HuddleCallAvatar({
    required this.pubkey,
    required this.profile,
    required this.fallbackLabel,
    required this.active,
    required this.speakerLevel,
    required this.onTap,
    this.isSelf = false,
    this.frameSize = _huddleAvatarFrameSize,
  });

  final String pubkey;
  final UserProfile? profile;
  final String? fallbackLabel;
  final bool active;
  final double speakerLevel;
  final VoidCallback? onTap;
  final bool isSelf;
  final double frameSize;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
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

    return SizedBox(
      width: frameSize,
      child: Semantics(
        label: active ? '$label, speaking' : label,
        hint: onTap == null ? null : 'Tap to focus participant',
        button: onTap != null,
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
                              shape: BoxShape.circle,
                              color: context.colors.primary.withValues(
                                alpha: 0.07,
                              ),
                            ),
                          ),
                        ),
                      ),
                      AvatarImage(
                        imageUrl: profile?.avatarUrl,
                        radius: avatarRadius,
                        backgroundColor: context.colors.primaryContainer,
                        fallback: Icon(
                          LucideIcons.userRound,
                          size: fallbackIconSize,
                          color: context.colors.onPrimaryContainer,
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
