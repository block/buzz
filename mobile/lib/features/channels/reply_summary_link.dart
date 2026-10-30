import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../../shared/theme/theme.dart';
import 'small_avatar.dart';
import 'timeline_message.dart';
import 'typing_dots.dart';

/// Desktop-style reply link with a curved branch below the message bubble.
class ReplySummaryLink extends ConsumerWidget {
  const ReplySummaryLink({
    super.key,
    required this.summary,
    required this.onTap,
    this.hasReactions = false,
    this.mirrored = false,
    this.working = const [],
    this.workingNames = const [],
  });

  /// Null while agents work on a message that has no replies yet.
  final ThreadSummary? summary;
  final VoidCallback onTap;
  final bool hasReactions;

  /// Agents working in this thread, shown with typing dots, as on desktop.
  final List<String> working;

  /// Display names for [working], in the same order, for the spoken label.
  final List<String> workingNames;

  /// Places the connector on the outgoing edge without reversing its contents.
  final bool mirrored;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final summary = this.summary;
    final workingKeys = {for (final pubkey in working) pubkey.toLowerCase()};
    // As on desktop, working agents leave the participant stack for the
    // working group: at most two avatars, then a count.
    final participants = [
      for (final pubkey in summary?.participantPubkeys ?? const <String>[])
        if (!workingKeys.contains(pubkey.toLowerCase())) pubkey,
    ].take(3).toList();
    final shownWorking = workingKeys.take(2).toList();
    final hiddenWorking = workingKeys.length - shownWorking.length;
    final userCache = ref.watch(userCacheProvider);
    final label = summary == null
        ? 'View thread'
        : summary.isCountPending
        ? 'Replies'
        : '${summary.replyCount}${summary.isLowerBound ? '+' : ''} ${summary.replyCount == 1 ? 'reply' : 'replies'}';
    // Desktop's spoken strings, in MessageRow's thread summary.
    final spoken = switch (workingNames) {
      [] => summary == null ? '' : label,
      _ when summary != null => '$label. ${workingNames.join(', ')} working',
      [final only] => '$only is working',
      _ => '${workingNames.length} agents are working',
    };
    final connectorTop = hasReactions ? -Grid.half : Grid.half;
    return Padding(
      padding: EdgeInsets.only(
        left: mirrored ? 0 : Grid.xxs,
        right: mirrored ? Grid.xxs : 0,
      ),
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          // As on desktop, the branch joins existing replies only.
          if (summary != null)
            Positioned.fill(
              top: connectorTop,
              child: IgnorePointer(
                child: CustomPaint(
                  painter: _ReplyConnector(
                    context.colors.outlineVariant,
                    mirrored ? TextDirection.rtl : TextDirection.ltr,
                    connectorTop,
                  ),
                ),
              ),
            ),
          Padding(
            padding: EdgeInsets.only(
              left: mirrored ? 0 : Grid.twelve,
              right: mirrored ? Grid.twelve : 0,
            ),
            child: Semantics(
              button: true,
              label: 'View thread: $spoken',
              onTap: onTap,
              excludeSemantics: true,
              child: InkWell(
                onTap: onTap,
                borderRadius: BorderRadius.circular(Radii.container),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(minHeight: 44),
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.half),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        if (participants.isNotEmpty) ...[
                          _AvatarStack(participants, userCache),
                          const SizedBox(width: Grid.half),
                        ],
                        if (shownWorking.isNotEmpty) ...[
                          // Working agents sit side by side, as on desktop.
                          for (final (i, pubkey) in shownWorking.indexed) ...[
                            if (i > 0) const SizedBox(width: Grid.xxs),
                            SmallAvatar(pubkey: pubkey, userCache: userCache),
                          ],
                          if (hiddenWorking > 0) ...[
                            const SizedBox(width: Grid.xxs),
                            Text(
                              '+$hiddenWorking',
                              style: context.textTheme.labelMedium?.copyWith(
                                color: context.colors.onSurfaceVariant,
                              ),
                            ),
                          ],
                          const SizedBox(width: Grid.xs),
                          const TypingDots(
                            key: ValueKey('thread-working-dots'),
                            dotSize: 5,
                          ),
                          const SizedBox(width: Grid.half),
                        ],
                        Flexible(
                          child: Text(
                            label,
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                            style: context.textTheme.labelMedium?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _AvatarStack extends StatelessWidget {
  const _AvatarStack(this.pubkeys, this.userCache);
  final List<String> pubkeys;
  final Map<String, UserProfile> userCache;

  @override
  Widget build(BuildContext context) => SizedBox(
    width: Grid.gutter + (pubkeys.length - 1) * Grid.twelve,
    height: Grid.gutter,
    child: Stack(
      children: [
        for (var i = 0; i < pubkeys.length; i++)
          PositionedDirectional(
            start: i * Grid.twelve,
            child: SmallAvatar(pubkey: pubkeys[i], userCache: userCache),
          ),
      ],
    ),
  );
}

class _ReplyConnector extends CustomPainter {
  const _ReplyConnector(this.color, this.direction, this.topOffset);
  final Color color;
  final TextDirection direction;
  final double topOffset;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    if (direction == TextDirection.rtl) {
      canvas.translate(size.width, 0);
      canvas.scale(-1, 1);
    }
    // Keep the endpoint aligned with the unchanged reply avatars.
    final middle = (size.height - topOffset) / 2;
    // Desktop uses a 12px outer corner with a 2px border: the stroke's
    // centerline follows an 11px circular arc, rather than a tight elbow.
    const radius = Grid.twelve - 1;
    final path = Path()
      ..moveTo(1, 0)
      ..lineTo(1, middle - radius)
      ..arcToPoint(
        Offset(Grid.twelve, middle),
        radius: const Radius.circular(radius),
        clockwise: false,
      );
    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(_ReplyConnector oldDelegate) =>
      color != oldDelegate.color ||
      direction != oldDelegate.direction ||
      topOffset != oldDelegate.topOffset;
}
