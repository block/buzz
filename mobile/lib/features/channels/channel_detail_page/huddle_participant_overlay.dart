part of '../channel_detail_page.dart';

const _huddleParticipantOverlayDuration = Duration(milliseconds: 320);
const _huddleParticipantSpotlightRadius = 68.0;

void _showHuddleParticipantSpotlight({
  required BuildContext context,
  required String ephemeralChannelId,
  required String pubkey,
  required bool isSelf,
}) {
  _showHuddleParticipantOverlay(
    context: context,
    barrierLabel: 'Dismiss participant',
    childBuilder: (dialogContext) => _HuddleParticipantSpotlight(
      ephemeralChannelId: ephemeralChannelId,
      pubkey: pubkey,
      isSelf: isSelf,
      onDismiss: () => Navigator.of(dialogContext).pop(),
    ),
  );
}

void _showHuddleParticipantRoster({
  required BuildContext context,
  required String ephemeralChannelId,
}) {
  _showHuddleParticipantOverlay(
    context: context,
    barrierLabel: 'Dismiss participant list',
    childBuilder: (_) =>
        _HuddleParticipantRoster(ephemeralChannelId: ephemeralChannelId),
  );
}

void _showHuddleParticipantOverlay({
  required BuildContext context,
  required String barrierLabel,
  required WidgetBuilder childBuilder,
}) {
  final reduceMotion = MediaQuery.disableAnimationsOf(context);
  unawaited(
    showGeneralDialog<void>(
      context: context,
      barrierDismissible: true,
      barrierLabel: barrierLabel,
      barrierColor: Colors.transparent,
      transitionDuration: reduceMotion
          ? Duration.zero
          : _huddleParticipantOverlayDuration,
      transitionBuilder: (context, animation, secondaryAnimation, child) =>
          child,
      pageBuilder: (dialogContext, animation, secondaryAnimation) =>
          _HuddleParticipantOverlay(
            animation: animation,
            child: childBuilder(dialogContext),
          ),
    ),
  );
}

class _HuddleParticipantOverlay extends StatelessWidget {
  const _HuddleParticipantOverlay({
    required this.animation,
    required this.child,
  });

  final Animation<double> animation;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: [
        Positioned.fill(
          child: AnimatedBuilder(
            animation: animation,
            builder: (context, child) {
              final blurProgress = const Interval(
                0,
                0.30,
                curve: Curves.easeOutCubic,
              ).transform(animation.value);
              final sigma = 20 * blurProgress;
              return BackdropFilter(
                key: const ValueKey('huddle-participant-modal-backdrop'),
                filter: ImageFilter.blur(sigmaX: sigma, sigmaY: sigma),
                child: ColoredBox(
                  color: context.colors.inverseSurface.withValues(
                    alpha: 0.10 * blurProgress,
                  ),
                ),
              );
            },
          ),
        ),
        Positioned.fill(
          child: GestureDetector(
            key: const ValueKey('huddle-participant-modal-dismiss-region'),
            behavior: HitTestBehavior.opaque,
            onTap: () => Navigator.of(context).pop(),
          ),
        ),
        SafeArea(
          child: Center(
            child: AnimatedBuilder(
              animation: animation,
              child: child,
              builder: (context, child) {
                final appearance = Curves.easeOutCubic.transform(
                  animation.value,
                );
                final scale = Curves.easeOutBack.transform(animation.value);
                return Opacity(
                  opacity: appearance,
                  child: Transform.scale(
                    scale: lerpDouble(0.92, 1, scale)!,
                    child: child,
                  ),
                );
              },
            ),
          ),
        ),
      ],
    );
  }
}

class _HuddleParticipantSpotlight extends ConsumerWidget {
  const _HuddleParticipantSpotlight({
    required this.ephemeralChannelId,
    required this.pubkey,
    required this.isSelf,
    required this.onDismiss,
  });

  final String ephemeralChannelId;
  final String pubkey;
  final bool isSelf;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final participantPresent =
        isSelf ||
        ref
            .watch(_huddleLogicalParticipantPubkeysProvider(ephemeralChannelId))
            .contains(pubkey);
    if (!participantPresent) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) onDismiss();
      });
      return const SizedBox.shrink();
    }
    final profile = ref.watch(
      userCacheProvider.select((profiles) => profiles[pubkey]),
    );
    final fallbackLabel = ref.watch(
      agentDirectoryDisplayNamesProvider.select((labels) => labels[pubkey]),
    );
    final active = ref.watch(
      huddleSessionProvider.select(
        (session) => session.activeSpeakerPubkeys.contains(pubkey),
      ),
    );
    final label = _huddleParticipantLabel(
      pubkey: pubkey,
      profile: profile,
      fallbackLabel: fallbackLabel,
      isSelf: isSelf,
    );
    final isAgent = profile?.isAgent == true || fallbackLabel != null;

    return Semantics(
      label: active ? '$label, speaking' : label,
      hint: 'Tap to close participant',
      button: true,
      excludeSemantics: true,
      child: GestureDetector(
        key: const ValueKey('huddle-participant-spotlight'),
        behavior: HitTestBehavior.opaque,
        excludeFromSemantics: true,
        onTap: onDismiss,
        child: Padding(
          padding: const EdgeInsets.all(Grid.gutter),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              AnimatedContainer(
                key: ValueKey('huddle-participant-spotlight-avatar-$pubkey'),
                duration: MediaQuery.disableAnimationsOf(context)
                    ? Duration.zero
                    : const Duration(milliseconds: 180),
                padding: EdgeInsets.all(active ? Grid.xxs : Grid.half),
                decoration: BoxDecoration(
                  shape: isAgent ? BoxShape.rectangle : BoxShape.circle,
                  borderRadius: isAgent
                      ? BorderRadius.circular(
                          (_huddleParticipantSpotlightRadius + Grid.half) * 0.6,
                        )
                      : null,
                  color: context.colors.primary.withValues(
                    alpha: active ? 0.18 : 0.08,
                  ),
                ),
                child: AvatarImage(
                  imageUrl: profile?.avatarUrl,
                  radius: _huddleParticipantSpotlightRadius,
                  backgroundColor: context.colors.primaryContainer,
                  fallback: Icon(
                    LucideIcons.userRound,
                    size: 56,
                    color: context.colors.onPrimaryContainer,
                  ),
                  isAgent: isAgent,
                ),
              ),
              const SizedBox(height: Grid.twelve),
              Material(
                color: context.colors.surface.withValues(alpha: 0.92),
                shape: const StadiumBorder(),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.xs,
                    vertical: Grid.xxs,
                  ),
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 280),
                    child: Text(
                      label,
                      key: const ValueKey('huddle-participant-spotlight-name'),
                      textAlign: TextAlign.center,
                      style: context.textTheme.titleMedium?.copyWith(
                        color: context.colors.onSurface,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
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

class _HuddleParticipantRoster extends ConsumerWidget {
  const _HuddleParticipantRoster({required this.ephemeralChannelId});

  final String ephemeralChannelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final session = ref.watch(huddleSessionProvider);
    final localPubkey = session.currentPubkey?.toLowerCase();
    final pubkeys = ref
        .watch(_huddleLogicalParticipantPubkeysProvider(ephemeralChannelId))
        .where((pubkey) => pubkey != localPubkey)
        .skip(_huddleClusterVisibleParticipantCount)
        .toList(growable: false);
    if (pubkeys.isEmpty) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) Navigator.of(context).pop();
      });
      return const SizedBox.shrink();
    }
    final profiles = ref.watch(userCacheProvider);
    final fallbackLabels = ref.watch(agentDirectoryDisplayNamesProvider);
    final activeSpeakerPubkeys = ref.watch(
      huddleSessionProvider.select((session) => session.activeSpeakerPubkeys),
    );
    final maxListHeight = min(MediaQuery.sizeOf(context).height * 0.55, 416.0);
    final listHeight = min(pubkeys.length * 64.0, maxListHeight);

    return Semantics(
      label: '${pubkeys.length} more participants',
      container: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () {},
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          child: Material(
            key: const ValueKey('huddle-participant-roster'),
            color: context.colors.surface,
            surfaceTintColor: Colors.transparent,
            elevation: 8,
            shadowColor: Colors.black.withValues(alpha: 0.2),
            borderRadius: BorderRadius.circular(Radii.dialog),
            clipBehavior: Clip.antiAlias,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 360),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(
                  Grid.gutter,
                  Grid.xs,
                  Grid.gutter,
                  Grid.xs,
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      '${pubkeys.length} more ${pubkeys.length == 1 ? 'person' : 'people'}',
                      textAlign: TextAlign.center,
                      style: context.textTheme.titleMedium?.copyWith(
                        color: context.colors.onSurface,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: Grid.twelve),
                    SizedBox(
                      height: listHeight,
                      child: ListView.separated(
                        key: const ValueKey('huddle-participant-roster-list'),
                        padding: EdgeInsets.zero,
                        itemCount: pubkeys.length,
                        separatorBuilder: (_, _) => Divider(
                          height: Grid.xxs,
                          color: context.colors.outlineVariant.withValues(
                            alpha: 0.5,
                          ),
                        ),
                        itemBuilder: (context, index) {
                          final pubkey = pubkeys[index];
                          final profile = profiles[pubkey];
                          final active = activeSpeakerPubkeys.contains(pubkey);
                          final label = _huddleParticipantLabel(
                            pubkey: pubkey,
                            profile: profile,
                            fallbackLabel: fallbackLabels[pubkey],
                            isSelf: false,
                          );
                          return Semantics(
                            label: active ? '$label, speaking' : label,
                            excludeSemantics: true,
                            child: SizedBox(
                              key: ValueKey(
                                'huddle-participant-roster-row-$pubkey',
                              ),
                              height: 56,
                              child: Row(
                                children: [
                                  AvatarImage(
                                    imageUrl: profile?.avatarUrl,
                                    radius: 22,
                                    backgroundColor:
                                        context.colors.primaryContainer,
                                    fallback: Icon(
                                      LucideIcons.userRound,
                                      size: 22,
                                      color: context.colors.onPrimaryContainer,
                                    ),
                                    isAgent:
                                        profile?.isAgent == true ||
                                        fallbackLabels[pubkey] != null,
                                  ),
                                  const SizedBox(width: Grid.twelve),
                                  Expanded(
                                    child: Text(
                                      label,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: context.textTheme.bodyLarge
                                          ?.copyWith(
                                            color: context.colors.onSurface,
                                            fontWeight: active
                                                ? FontWeight.w600
                                                : null,
                                          ),
                                    ),
                                  ),
                                ],
                              ),
                            ),
                          );
                        },
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
