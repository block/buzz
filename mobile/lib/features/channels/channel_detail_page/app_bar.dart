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
                        BuzzIcons.lock,
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
    presentedPresenceProvider.select(
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

/// One-tap entry to a working agent's live transcript (#3907).
///
/// The members badge alone was easy to miss; this dedicated control opens
/// [AgentActivitySheet] for the first typing bot without an extra hop through
/// the members sheet. It is omitted unless an agent in this channel is working.
class _AgentActivityButton extends ConsumerWidget {
  const _AgentActivityButton({required this.channelId});

  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final action = _agentActivityNavigationAction(context, ref, channelId);
    if (action == null) return const SizedBox.shrink();

    return IconButton(
      color: context.colors.primary,
      onPressed: action.onPressed,
      tooltip: action.label,
      icon: Stack(
        clipBehavior: Clip.none,
        children: [
          const Icon(BuzzIcons.activity, size: 22),
          Positioned(
            top: -2,
            right: -2,
            child: Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(
                color: context.appColors.success,
                shape: BoxShape.circle,
                border: Border.all(color: context.colors.surface, width: 1.5),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Header action for [channelId], or null when no agent there is working.
///
/// The native bar reads nativeActions and ignores the Flutter actions list
/// when that list is set, so the shortcut has to be built for both.
IosNavigationAction? _agentActivityNavigationAction(
  BuildContext context,
  WidgetRef ref,
  String channelId,
) {
  final working = ref.watch(workingBotPubkeysProvider(channelId));
  if (working.isEmpty) return null;
  final agentPubkey = working.first;
  return IosNavigationAction(
    label: 'View agent activity',
    symbol: 'waveform.path.ecg',
    icon: BuzzIcons.activity,
    activityColor: context.appColors.success,
    activityLabel: 'Agent working',
    onPressed: () {
      showAgentActivitySheet(
        context: context,
        channelId: channelId,
        agentPubkey: agentPubkey,
      );
    },
  );
}
