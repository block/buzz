import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/animated_avatar.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/relay/media_image.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/anchored_popover_menu.dart';
import '../../shared/widgets/progressive_animated_avatar.dart';
import 'profile_provider.dart';
import 'ios_presence_menu.dart';
import 'set_status_sheet.dart';
import 'user_status_provider.dart';

/// Profile avatar, display name, status, and presence shown above settings.
class SettingsProfileHeader extends HookConsumerWidget {
  const SettingsProfileHeader({super.key});

  static const _avatarSize = 128.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profile = ref.watch(profileProvider).asData?.value;
    final status = ref.watch(userStatusProvider).asData?.value;
    final hasStatus = status != null && !status.isEmpty;
    final palette = ref.watch(customEmojiListProvider);
    final shortcode = normalizeShortcode(status?.emoji ?? '');
    final customEmoji = palette
        .where((entry) => entry.shortcode == shortcode)
        .firstOrNull;
    final presence = ref.watch(presenceProvider).value ?? 'offline';
    final animatedAvatar = parseAnimatedAvatarUrl(profile?.avatarUrl);
    final animatedPosterUrl = animatedAvatar?.posterUrl;
    final handoff = ref.watch(profileAvatarHandoffProvider);
    final activeHandoff = handoff?.avatarUrl == profile?.avatarUrl
        ? handoff
        : null;
    final stoppedAnimationUrl = useState<String?>(null);
    final avatarUrl = animatedAvatar == null
        ? profile?.avatarUrl
        : stoppedAnimationUrl.value == animatedAvatar.animationUrl
        ? animatedAvatar.posterUrl
        : null;

    void openStatusSheet() =>
        showSetStatusSheet(context, currentStatus: status);

    return Padding(
      key: const ValueKey('settings-profile-header'),
      padding: const EdgeInsets.only(top: Grid.sm, bottom: Grid.twelve),
      child: Column(
        children: [
          SizedBox.square(
            dimension: _avatarSize,
            child: ClipOval(
              child: GestureDetector(
                key: const ValueKey('settings-profile-avatar'),
                onTap: animatedAvatar == null
                    ? null
                    : () => stoppedAnimationUrl.value =
                          stoppedAnimationUrl.value ==
                              animatedAvatar.animationUrl
                          ? null
                          : animatedAvatar.animationUrl,
                child: ColoredBox(
                  key: const ValueKey('settings-profile-avatar-background'),
                  color: animatedAvatar == null
                      ? context.colors.primaryContainer
                      : Colors.transparent,
                  child:
                      animatedAvatar != null &&
                          stoppedAnimationUrl.value !=
                              animatedAvatar.animationUrl
                      ? ProgressiveAnimatedAvatar(
                          key: ValueKey(animatedAvatar.animationUrl),
                          descriptor: animatedAvatar,
                          fallback: _AvatarFallback(initial: profile?.initial),
                          loadingImage: activeHandoff == null
                              ? null
                              : MemoryImage(activeHandoff.animation),
                          onAnimationReady: activeHandoff == null
                              ? null
                              : () => ref
                                    .read(profileAvatarHandoffProvider.notifier)
                                    .clear(activeHandoff.avatarUrl),
                        )
                      : activeHandoff == null || animatedPosterUrl == null
                      ? AvatarImageContent(
                          imageUrl: avatarUrl,
                          fallback: _AvatarFallback(initial: profile?.initial),
                        )
                      : Stack(
                          fit: StackFit.expand,
                          children: [
                            Image(
                              image: MemoryImage(activeHandoff.poster),
                              fit: BoxFit.cover,
                              gaplessPlayback: true,
                            ),
                            Offstage(
                              offstage: true,
                              child: MediaImage(
                                key: ValueKey(
                                  'settings-profile-paused-handoff-${activeHandoff.avatarUrl}',
                                ),
                                url: animatedPosterUrl,
                                fit: BoxFit.cover,
                                errorBuilder: (_, _, _) =>
                                    const SizedBox.shrink(),
                                frameBuilder:
                                    (
                                      context,
                                      child,
                                      frame,
                                      wasSynchronouslyLoaded,
                                    ) {
                                      if (wasSynchronouslyLoaded ||
                                          frame != null) {
                                        WidgetsBinding.instance
                                            .addPostFrameCallback((_) {
                                              ref
                                                  .read(
                                                    profileAvatarHandoffProvider
                                                        .notifier,
                                                  )
                                                  .clear(
                                                    activeHandoff.avatarUrl,
                                                  );
                                            });
                                      }
                                      return child;
                                    },
                              ),
                            ),
                          ],
                        ),
                ),
              ),
            ),
          ),
          const SizedBox(height: Grid.twelve),
          Text(
            profile?.label ?? 'Your profile',
            style: context.textTheme.titleMedium,
            textAlign: TextAlign.center,
          ),
          // Preserve the complete status below the name, including custom emoji.
          if (hasStatus)
            GestureDetector(
              onTap: openStatusSheet,
              child: Padding(
                padding: const EdgeInsets.only(
                  top: Grid.quarter,
                  left: Grid.gutter,
                  right: Grid.gutter,
                  bottom: Grid.half,
                ),
                child: Text.rich(
                  TextSpan(
                    children: [
                      if (customEmoji != null)
                        WidgetSpan(
                          alignment: PlaceholderAlignment.middle,
                          child: CustomEmojiImage(
                            shortcode: customEmoji.shortcode,
                            url: customEmoji.url,
                          ),
                        )
                      else if (status.emoji.isNotEmpty)
                        TextSpan(text: status.emoji),
                      if (status.emoji.isNotEmpty && status.text.isNotEmpty)
                        const TextSpan(text: ' '),
                      if (status.text.isNotEmpty) TextSpan(text: status.text),
                    ],
                  ),
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                  textAlign: TextAlign.center,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          _PresencePill(
            presence: presence,
            onSelected: (nextPresence) => unawaited(
              ref.read(presenceProvider.notifier).setPresence(nextPresence),
            ),
          ),
        ],
      ),
    );
  }
}

class _AvatarFallback extends StatelessWidget {
  const _AvatarFallback({required this.initial});

  final String? initial;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Text(
        initial ?? '?',
        style: context.textTheme.displaySmall?.copyWith(
          color: context.colors.onPrimaryContainer,
        ),
      ),
    );
  }
}

class _PresencePill extends StatelessWidget {
  const _PresencePill({required this.presence, required this.onSelected});

  final String presence;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) {
    final effectivePresence = switch (presence) {
      'online' || 'away' => presence,
      _ => 'offline',
    };
    final (backgroundColor, foregroundColor) = switch (effectivePresence) {
      'online' => (
        context.appColors.success.withValues(alpha: 0.15),
        context.appColors.success,
      ),
      'away' => (
        context.appColors.warning.withValues(alpha: 0.15),
        context.appColors.warning,
      ),
      _ => (
        context.colors.onSurfaceVariant.withValues(alpha: 0.15),
        context.colors.onSurfaceVariant,
      ),
    };
    final label = _presenceLabel(effectivePresence);

    if (!kIsWeb && defaultTargetPlatform == TargetPlatform.iOS) {
      return IosPresenceMenu(
        presence: effectivePresence,
        label: label,
        foreground: foregroundColor,
        background: backgroundColor,
        onSelected: onSelected,
      );
    }

    return Builder(
      builder: (buttonContext) => Semantics(
        button: true,
        label: 'Presence: $label',
        child: SizedBox(
          key: const ValueKey('settings-presence-target'),
          height: Grid.xl,
          child: Material(
            color: Colors.transparent,
            child: InkWell(
              key: const ValueKey('settings-presence-menu'),
              borderRadius: BorderRadius.circular(Radii.full),
              onTap: () async {
                unawaited(HapticFeedback.selectionClick());
                final selected = await showAnchoredPopover<String>(
                  context: buttonContext,
                  width: 176,
                  alignment: AnchoredPopoverAlignment.center,
                  offset: const Offset(0, Grid.half),
                  menuPadding: const EdgeInsets.symmetric(vertical: Grid.half),
                  surfaceKey: const ValueKey('settings-presence-popover'),
                  items: [
                    for (final option in const ['online', 'away', 'offline'])
                      PopupMenuItem<String>(
                        key: ValueKey('settings-presence-$option'),
                        value: option,
                        height: Grid.xl,
                        padding: const EdgeInsets.symmetric(
                          horizontal: Grid.twelve,
                        ),
                        child: Row(
                          children: [
                            Container(
                              width: 10,
                              height: 10,
                              decoration: BoxDecoration(
                                color: _presenceColor(context, option),
                                shape: BoxShape.circle,
                              ),
                            ),
                            const SizedBox(width: Grid.xxs),
                            Expanded(
                              child: Text(
                                _presenceLabel(option),
                                style: filterChipTextStyle.copyWith(
                                  color: context.colors.onSurface,
                                  fontWeight: option == effectivePresence
                                      ? FontWeight.w500
                                      : FontWeight.w400,
                                ),
                              ),
                            ),
                            if (option == effectivePresence)
                              Icon(
                                BuzzIcons.check,
                                size: 16,
                                color: context.colors.primary,
                              ),
                          ],
                        ),
                      ),
                  ],
                );
                if (buttonContext.mounted && selected != null) {
                  onSelected(selected);
                }
              },
              child: Center(
                child: Material(
                  key: const ValueKey('settings-presence-pill'),
                  color: backgroundColor,
                  borderRadius: BorderRadius.circular(Radii.full),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.xs,
                      vertical: Grid.xxs,
                    ),
                    child: Text(
                      label,
                      key: const ValueKey('settings-presence-label'),
                      style: filterChipTextStyle.copyWith(
                        color: foregroundColor,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

String _presenceLabel(String presence) => switch (presence) {
  'online' => 'Online',
  'away' => 'Away',
  _ => 'Offline',
};

Color _presenceColor(BuildContext context, String presence) =>
    switch (presence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.outline,
    };
