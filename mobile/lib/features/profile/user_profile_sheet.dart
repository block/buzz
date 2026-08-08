import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import '../channels/message_content.dart';
import 'presence_cache_provider.dart';
import 'user_cache_provider.dart';
import 'user_status_cache_provider.dart';

/// Show a user profile bottom sheet for the given [pubkey].
void showUserProfileSheet(BuildContext context, String pubkey) {
  showBuzzModalBottomSheet<Channel>(
    context: context,
    isScrollControlled: true,
    showDragHandle: false,
    builder: (_) => UserProfileSheet(pubkey: pubkey),
  ).then((channel) {
    if (channel == null || !context.mounted) return;
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ChannelDetailPage(channel: channel),
      ),
    );
  });
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
    final copied = useState(false);
    final isOpeningDirectMessage = useState(false);

    final displayName = profile?.displayName;
    final avatarUrl = profile?.avatarUrl;
    final nip05 = profile?.nip05Handle;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');

    Future<void> copyPublicKey() async {
      await Clipboard.setData(ClipboardData(text: pubkey));
      if (!context.mounted) return;
      copied.value = true;
      _showProfileCopyToast(context);
      unawaited(
        Future<void>.delayed(const Duration(seconds: 2), () {
          if (context.mounted) copied.value = false;
        }),
      );
    }

    Future<void> openDirectMessage() async {
      if (isOpeningDirectMessage.value) return;
      isOpeningDirectMessage.value = true;
      try {
        final channel = await ref
            .read(channelActionsProvider)
            .openDm(pubkeys: [pk]);
        if (!context.mounted) return;
        Navigator.of(context).pop(channel);
      } catch (_) {
        // Silently fail — user tapped but DM open failed.
        if (context.mounted) isOpeningDirectMessage.value = false;
      }
    }

    return ConstrainedBox(
      constraints: BoxConstraints(
        maxHeight: MediaQuery.sizeOf(context).height * 0.7,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Flexible(
            child: Padding(
              padding: EdgeInsets.fromLTRB(
                Grid.gutter,
                0,
                Grid.gutter,
                MediaQuery.viewInsetsOf(context).bottom,
              ),
              child: SingleChildScrollView(
                padding: const EdgeInsets.only(bottom: Grid.xs),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    // Center the presence chip on the avatar's lower edge.
                    Padding(
                      padding: const EdgeInsets.only(bottom: Grid.xl / 2),
                      child: Stack(
                        clipBehavior: Clip.none,
                        children: [
                          Align(
                            alignment: Alignment.center,
                            child: FractionallySizedBox(
                              widthFactor: 0.5,
                              child: _ProfileAvatar(
                                avatarUrl: avatarUrl,
                                initial: initial,
                              ),
                            ),
                          ),
                          Positioned(
                            left: 0,
                            right: 0,
                            bottom: -(Grid.xl / 2),
                            child: _ProfilePresenceChip(presence: presence),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(height: Grid.half),

                    // Display name — centered, large
                    Center(
                      child: Text(
                        displayName ?? shortPubkey(pubkey),
                        style: context.textTheme.headlineSmall?.copyWith(
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                    ),
                    // Match Settings: status is quiet, centered copy directly
                    // below the profile name rather than a separate information row.
                    if (userStatus != null && !userStatus.isEmpty)
                      SizedBox(
                        width: double.infinity,
                        child: Padding(
                          padding: const EdgeInsets.only(
                            top: Grid.xxs,
                            left: Grid.gutter,
                            right: Grid.gutter,
                            bottom: Grid.xxs,
                          ),
                          child: MessageContent(
                            content: userStatus.text.isNotEmpty
                                ? '${userStatus.emoji.isNotEmpty ? '${userStatus.emoji} ' : ''}${userStatus.text}'
                                : userStatus.emoji,
                            baseStyle: context.textTheme.bodySmall?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                            textAlign: TextAlign.center,
                            maxLines: 2,
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

                    const SizedBox(height: Grid.xs + Grid.half),

                    Row(
                      children: [
                        if (pk != currentPubkey) ...[
                          Expanded(
                            child: _ProfileActionTile(
                              icon: isOpeningDirectMessage.value
                                  ? null
                                  : LucideIcons.messageSquare,
                              label: isOpeningDirectMessage.value
                                  ? 'Opening…'
                                  : 'Message',
                              isLoading: isOpeningDirectMessage.value,
                              onTap: openDirectMessage,
                            ),
                          ),
                          const SizedBox(width: Grid.twelve),
                        ],
                        Expanded(
                          child: _ProfileActionTile(
                            icon: copied.value
                                ? LucideIcons.check
                                : LucideIcons.key,
                            label: copied.value ? 'Copied' : 'Copy public key',
                            onTap: copyPublicKey,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: Grid.xs),

                    // About / bio section
                    if (about.isNotEmpty) ...[
                      const SizedBox(height: Grid.xxs),
                      Divider(
                        color: context.colors.outlineVariant.withValues(
                          alpha: 0.3,
                        ),
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
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Shows clipboard feedback above the bottom-sheet route. A regular SnackBar
/// belongs to the page Scaffold, which sits behind the modal sheet.
void _showProfileCopyToast(BuildContext context) {
  final overlay = Overlay.of(context, rootOverlay: true);
  final colors = context.colors;
  final textStyle = context.textTheme.bodyMedium?.copyWith(
    color: colors.onInverseSurface,
  );
  late final OverlayEntry entry;
  entry = OverlayEntry(
    builder: (overlayContext) => Positioned(
      left: Grid.gutter,
      right: Grid.gutter,
      bottom: MediaQuery.viewPaddingOf(overlayContext).bottom + Grid.xs,
      child: Material(
        color: colors.inverseSurface,
        elevation: 6,
        borderRadius: BorderRadius.circular(Radii.lg),
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xs,
            vertical: Grid.twelve,
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(LucideIcons.check, size: 18, color: colors.onInverseSurface),
              const SizedBox(width: Grid.xxs),
              Text('Public key copied', style: textStyle),
            ],
          ),
        ),
      ),
    ),
  );
  overlay.insert(entry);
  Future<void>.delayed(const Duration(seconds: 2), () {
    if (entry.mounted) entry.remove();
  });
}

/// The read-only version of the presence control in Settings. A viewed profile
/// reflects its owner's availability but does not allow another user to change
/// it.
class _ProfilePresenceChip extends StatelessWidget {
  const _ProfilePresenceChip({required this.presence});

  final String presence;

  @override
  Widget build(BuildContext context) {
    final effectivePresence = switch (presence) {
      'online' || 'away' => presence,
      _ => 'offline',
    };
    final backgroundColor = switch (effectivePresence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.onSurfaceVariant,
    };
    final label = switch (effectivePresence) {
      'online' => 'Online',
      'away' => 'Away',
      _ => 'Offline',
    };

    return Semantics(
      label: 'Presence: $label',
      child: SizedBox(
        height: Grid.xl,
        child: Center(
          child: Material(
            color: backgroundColor,
            borderRadius: BorderRadius.circular(Radii.full),
            child: Padding(
              padding: const EdgeInsets.symmetric(
                horizontal: Grid.xs,
                vertical: Grid.xxs,
              ),
              child: Text(
                label,
                style: filterChipTextStyle.copyWith(
                  color: context.colors.onPrimary,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ProfileActionTile extends StatelessWidget {
  const _ProfileActionTile({
    required this.icon,
    required this.label,
    required this.onTap,
    this.isLoading = false,
  });

  final IconData? icon;
  final String label;
  final VoidCallback onTap;
  final bool isLoading;

  @override
  Widget build(BuildContext context) => GestureDetector(
    onTap: isLoading
        ? null
        : () {
            unawaited(HapticFeedback.lightImpact());
            onTap();
          },
    behavior: HitTestBehavior.opaque,
    child: Container(
      width: double.infinity,
      height: 68 + (Grid.xxs * 2),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.dialog),
      ),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          if (isLoading)
            BuzzLoadingIndicator(
              size: 22,
              color: context.colors.onSurface,
              semanticLabel: 'Opening direct message',
            )
          else
            Icon(icon, size: 22, color: context.colors.onSurface),
          const SizedBox(height: Grid.xxs),
          Text(
            label,
            style: context.textTheme.labelMedium?.copyWith(
              color: context.colors.onSurface,
            ),
          ),
        ],
      ),
    ),
  );
}

class _ProfileAvatar extends StatelessWidget {
  final String? avatarUrl;
  final String initial;

  const _ProfileAvatar({required this.avatarUrl, required this.initial});

  @override
  Widget build(BuildContext context) {
    return AspectRatio(
      aspectRatio: 1,
      child: ClipOval(
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
