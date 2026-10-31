import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../theme/theme.dart';
import 'community_avatar.dart';

import 'community_grid.dart';

/// A community tile shared by onboarding and the switcher.
class CommunityGridOption extends HookWidget {
  const CommunityGridOption({
    super.key,
    required this.id,
    required this.name,
    required this.relayUrl,
    required this.onSelect,
    this.isActive = false,
    this.hidden = false,
    this.enabled = true,
    this.badge,
  });
  final String id;
  final String name;
  final String relayUrl;
  final Future<void> Function(Rect) onSelect;
  final bool isActive;
  final bool hidden;
  final bool enabled;
  final Widget? badge;
  @override
  Widget build(BuildContext context) {
    final avatarKey = useMemoized(GlobalKey.new);
    return Semantics(
      selected: isActive,
      button: enabled,
      child: InkWell(
        key: Key('community-switcher-row-$id'),
        borderRadius: BorderRadius.circular(Radii.card),
        onTap: !enabled
            ? null
            : () {
                final box = avatarKey.currentContext?.findRenderObject();
                if (box is RenderBox) {
                  unawaited(
                    onSelect(box.localToGlobal(Offset.zero) & box.size),
                  );
                }
              },
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
          child: Column(
            children: [
              Stack(
                clipBehavior: Clip.none,
                children: [
                  Opacity(
                    opacity: hidden ? 0 : 1,
                    child: SizedBox(
                      key: avatarKey,
                      child: ExcludeSemantics(
                        child: CommunityAvatar(
                          key: Key('community-switcher-avatar-$id'),
                          name: name,
                          relayUrl: relayUrl,
                          size: communityGridAvatarSize,
                        ),
                      ),
                    ),
                  ),
                  if (badge != null)
                    Positioned(
                      right: -Grid.twelve,
                      bottom: -Grid.twelve,
                      child: badge!,
                    ),
                ],
              ),
              const SizedBox(height: Grid.xs),
              Text(
                name,
                textAlign: TextAlign.center,
                style: context.textTheme.bodyLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
