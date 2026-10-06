part of '../channel_detail_page.dart';

double _scaledTextHeight(BuildContext context, TextStyle style) {
  return MediaQuery.textScalerOf(context).scale(style.fontSize ?? 0) *
      (style.height ?? 1);
}

double _twoLineAppBarTitleContentHeight(BuildContext context) {
  final titleStyle = context.textTheme.titleSmall;
  final subtitleStyle = context.textTheme.bodySmall;
  if (titleStyle == null || subtitleStyle == null) return 40;
  return max(
    40,
    _scaledTextHeight(context, titleStyle) +
        _scaledTextHeight(context, subtitleStyle),
  );
}

class _ConversationAppBarTitle extends StatelessWidget {
  const _ConversationAppBarTitle({
    required this.channel,
    required this.label,
    required this.subtitle,
    required this.presence,
    required this.onTap,
  });

  final Channel channel;
  final String label;
  final String subtitle;
  final String? presence;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final prefix = channel.isDm ? 'dm' : 'channel';
    return Semantics(
      button: true,
      child: Tooltip(
        message: channel.isDm
            ? 'Open conversation details'
            : 'Open channel settings',
        child: InkWell(
          key: ValueKey('$prefix-header-settings-trigger'),
          borderRadius: BorderRadius.circular(Radii.md),
          onTap: onTap,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 48),
            child: Column(
              key: ValueKey('$prefix-header-text-stack'),
              mainAxisSize: MainAxisSize.min,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (!channel.isDm && channel.visibility == 'private') ...[
                      Icon(
                        LucideIcons.lock,
                        size: 14,
                        color: context.colors.onSurfaceVariant,
                        semanticLabel: 'Private channel',
                      ),
                      const SizedBox(width: Grid.quarter),
                    ],
                    Flexible(
                      child: Text(
                        label,
                        key: ValueKey('$prefix-header-name'),
                        textAlign: TextAlign.center,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.titleSmall?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    if (channel.isEphemeral) ...[
                      const SizedBox(width: Grid.quarter),
                      _HeaderEphemeralBadge(channel: channel),
                    ],
                  ],
                ),
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (presence != null) ...[
                      ExcludeSemantics(
                        child: Container(
                          key: const ValueKey('dm-header-presence-dot'),
                          width: 6,
                          height: 6,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: switch (presence) {
                              'online' => context.appColors.success,
                              'away' => context.appColors.warning,
                              _ => context.colors.outline,
                            },
                          ),
                        ),
                      ),
                      const SizedBox(width: Grid.quarter),
                    ],
                    Flexible(
                      child: Text(
                        subtitle,
                        key: ValueKey(
                          channel.isDm
                              ? 'dm-header-presence'
                              : 'channel-header-member-count',
                        ),
                        textAlign: TextAlign.center,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.bodySmall?.copyWith(
                          color: context.colors.onSurface.withValues(
                            alpha: 0.65,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

// Both renderers subscribe to the same counterpart identity and presence.
({String label, String? presence, String presenceLabel}) _watchDmHeader(
  WidgetRef ref,
  Channel channel,
  String? currentPubkey,
) {
  final normalizedCurrent = currentPubkey?.toLowerCase();

  String? otherPubkey;
  for (final pk in channel.participantPubkeys) {
    if (pk.toLowerCase() != normalizedCurrent) {
      otherPubkey = pk.toLowerCase();
      break;
    }
  }

  final profile = ref.watch(
    userCacheProvider.select(
      (profiles) => otherPubkey == null ? null : profiles[otherPubkey],
    ),
  );
  final presence = ref.watch(
    presenceCacheProvider.select(
      (presenceMap) => otherPubkey == null ? null : presenceMap[otherPubkey],
    ),
  );

  if (otherPubkey != null) {
    if (profile == null) {
      ref.read(userCacheProvider.notifier).preload([otherPubkey]);
    }
    ref.read(presenceCacheProvider.notifier).track([otherPubkey]);
  }

  final presenceLabel = switch (presence) {
    'online' => 'Online',
    'away' => 'Away',
    'offline' => 'Offline',
    _ => 'Unknown',
  };

  return (
    label: ref.watch(
      identityNameSourcesProvider.select(
        (names) => resolveDmChannelDisplayLabel(
          channel,
          currentPubkey: currentPubkey,
          names: names,
        ),
      ),
    ),
    presence: presence,
    presenceLabel: presenceLabel,
  );
}
