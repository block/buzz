import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/utils/string_utils.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import 'presence_cache_provider.dart';
import 'user_cache_provider.dart';
import 'user_status_cache_provider.dart';
import '../channels/message_content.dart';

/// Show a user profile bottom sheet for the given [pubkey].
void showUserProfileSheet(BuildContext context, String pubkey) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => UserProfileSheet(pubkey: pubkey),
  );
}

class UserProfileSheet extends HookConsumerWidget {
  final String pubkey;

  const UserProfileSheet({super.key, required this.pubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = pubkey.toLowerCase();
    final currentPubkey = ref.watch(currentPubkeyProvider);

    // Watch cached profile, presence, and user status.
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final presenceMap = ref.watch(presenceCacheProvider);
    final presence = presenceMap[pk] ?? 'offline';
    final statusCache = ref.watch(userStatusCacheProvider);
    final userStatus = statusCache[pk];

    // Fetch about from the user's kind:0 profile event.
    final aboutFuture = useMemoized(
      () => ref
          .read(relaySessionProvider.notifier)
          .fetchHistory(NostrFilters.profile(pk))
          .then((events) {
            if (events.isEmpty) return '';
            return ProfileData.fromEvent(events.first).about ?? '';
          })
          .catchError((_) => ''),
      [pk],
    );
    final aboutSnapshot = useFuture(aboutFuture);
    final about = aboutSnapshot.data ?? profile?.about ?? '';

    // Ensure presence and status are tracked.
    useEffect(() {
      ref.read(presenceCacheProvider.notifier).track([pk]);
      ref.read(userStatusCacheProvider.notifier).track([pk]);
      ref.read(userCacheProvider.notifier).preload([pk]);
      return null;
    }, [pk]);

    final displayName = profile?.displayName;
    final avatarUrl = profile?.avatarUrl;
    final nip05 = profile?.nip05Handle;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');

    final presenceColor = switch (presence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.outline,
    };
    final presenceLabel = switch (presence) {
      'online' => 'Online',
      'away' => 'Away',
      _ => 'Offline',
    };

    return SizedBox(
      width: double.infinity,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          Grid.sm,
          0,
          Grid.sm,
          MediaQuery.viewInsetsOf(context).bottom,
        ),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * 0.7,
          ),
          child: SingleChildScrollView(
            padding: const EdgeInsets.only(bottom: Grid.xs),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // Avatar — near full-width
                _ProfileAvatar(avatarUrl: avatarUrl, initial: initial),
                const SizedBox(height: Grid.xs),

                // Display name — centered, large
                Center(
                  child: Text(
                    displayName ?? shortPubkey(pubkey),
                    style: context.textTheme.headlineSmall?.copyWith(
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),

                // NIP-05 handle — centered, secondary
                if (nip05 != null && nip05.isNotEmpty) ...[
                  const SizedBox(height: Grid.half),
                  Center(
                    child: Text(
                      nip05,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ),
                ],

                const SizedBox(height: Grid.xs),

                // Presence row — filled dot, not an outline icon
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: Grid.half + 2),
                  child: Row(
                    children: [
                      SizedBox(
                        width: 24,
                        child: Center(
                          child: Container(
                            width: 10,
                            height: 10,
                            decoration: BoxDecoration(
                              color: presenceColor,
                              shape: BoxShape.circle,
                            ),
                          ),
                        ),
                      ),
                      const SizedBox(width: Grid.xxs),
                      Text(
                        presenceLabel,
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurface,
                        ),
                      ),
                    ],
                  ),
                ),

                if (userStatus != null && !userStatus.isEmpty)
                  _InfoRowContent(
                    icon: LucideIcons.messageCircle,
                    child: MessageContent(
                      content:
                          '${userStatus.emoji.isNotEmpty ? '${userStatus.emoji} ' : ''}${userStatus.text}',
                    ),
                  ),

                _InfoRow(
                  icon: LucideIcons.key,
                  text: shortPubkey(pubkey),
                  textStyle: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                    fontFamily: 'monospace',
                  ),
                  onTap: () async {
                    await copyToClipboard(
                      context,
                      pubkey,
                      message: 'Public key copied',
                    );
                  },
                ),

                // About / bio section
                if (about.isNotEmpty) ...[
                  const SizedBox(height: Grid.xxs),
                  Divider(
                    color: context.colors.outlineVariant.withValues(alpha: 0.3),
                  ),
                  const SizedBox(height: Grid.xxs),
                  Text(
                    'About',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: Grid.half),
                  Text(
                    about,
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ],

                const SizedBox(height: Grid.xs),

                // Action button — Message (hidden on own profile)
                if (pk != currentPubkey) ...[
                  SizedBox(
                    width: double.infinity,
                    child: FilledButton.icon(
                      onPressed: () async {
                        Navigator.of(context).pop();
                        try {
                          final channel = await ref
                              .read(channelActionsProvider)
                              .openDm(pubkeys: [pk]);
                          if (!context.mounted) return;
                          await Navigator.of(context).push(
                            MaterialPageRoute<void>(
                              builder: (_) =>
                                  ChannelDetailPage(channel: channel),
                            ),
                          );
                        } catch (_) {
                          // Silently fail — user tapped but DM open failed.
                        }
                      },
                      icon: const Icon(LucideIcons.messageSquare, size: 18),
                      label: const Text('Message'),
                      style: FilledButton.styleFrom(
                        padding: const EdgeInsets.symmetric(
                          vertical: Grid.twelve,
                        ),
                        shape: RoundedRectangleBorder(
                          borderRadius: BorderRadius.circular(Radii.lg),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(height: Grid.xxs),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A row displaying an icon + text, used for profile info items.

class _InfoRowContent extends StatelessWidget {
  final IconData icon;
  final Widget child;

  const _InfoRowContent({required this.icon, required this.child});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.quarter),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, size: 16, color: context.colors.onSurfaceVariant),
          const SizedBox(width: Grid.xxs),
          Expanded(child: child),
        ],
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final IconData icon;
  final String text;
  final TextStyle? textStyle;
  final VoidCallback? onTap;

  const _InfoRow({
    required this.icon,
    required this.text,
    this.textStyle,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final child = Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half + 2),
      child: Row(
        children: [
          SizedBox(
            width: 24,
            child: Icon(icon, size: 16, color: context.colors.onSurfaceVariant),
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Text(
              text,
              style:
                  textStyle ??
                  context.textTheme.bodyMedium?.copyWith(
                    color: context.colors.onSurface,
                  ),
            ),
          ),
          if (onTap != null)
            Icon(
              LucideIcons.copy,
              size: 14,
              color: context.colors.onSurfaceVariant,
            ),
        ],
      ),
    );

    if (onTap != null) {
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: child,
      );
    }
    return child;
  }
}

class _ProfileAvatar extends StatelessWidget {
  final String? avatarUrl;
  final String initial;

  const _ProfileAvatar({required this.avatarUrl, required this.initial});

  @override
  Widget build(BuildContext context) {
    return AspectRatio(
      aspectRatio: 1,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(20),
        child: AvatarImageContent(
          imageUrl: avatarUrl,
          fallback: _AvatarFallback(initial: initial),
        ),
      ),
    );
  }
}

class _AvatarFallback extends StatelessWidget {
  final String initial;

  const _AvatarFallback({required this.initial});

  @override
  Widget build(BuildContext context) {
    return Container(
      color: context.colors.primaryContainer,
      alignment: Alignment.center,
      child: Text(
        initial,
        style: context.textTheme.displayLarge?.copyWith(
          color: context.colors.onPrimaryContainer,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}
