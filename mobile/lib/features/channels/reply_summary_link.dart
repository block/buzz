import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
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
    this.workingLabel,
  });

  /// Null while agents work on a message that has no replies yet.
  final ThreadSummary? summary;
  final VoidCallback onTap;
  final bool hasReactions;

  /// Agents working in this thread, shown first with typing dots, as on desktop.
  final List<String> working;

  /// Spoken description of [working], such as "Morty is working".
  final String? workingLabel;

  /// Places the connector on the outgoing edge without reversing its contents.
  final bool mirrored;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final summary = this.summary;
    final participants = {
      for (final pubkey in working) pubkey.toLowerCase(),
      for (final pubkey in summary?.participantPubkeys ?? const <String>[])
        pubkey.toLowerCase(),
    }.take(3).toList();
    final userCache = ref.watch(userCacheProvider);
    final label = summary == null
        ? null
        : summary.isCountPending
        ? 'Replies'
        : '${summary.replyCount}${summary.isLowerBound ? '+' : ''} ${summary.replyCount == 1 ? 'reply' : 'replies'}';
    final spoken = [?label, if (working.isNotEmpty) ?workingLabel].join(', ');
    final connectorTop = hasReactions ? -Grid.half : Grid.half;
    return Padding(
      padding: EdgeInsets.only(
        left: mirrored ? 0 : Grid.xxs,
        right: mirrored ? Grid.xxs : 0,
      ),
      child: Stack(
        clipBehavior: Clip.none,
        children: [
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
                          SizedBox(
                            width:
                                Grid.gutter +
                                (participants.length - 1) * Grid.twelve,
                            height: Grid.gutter,
                            child: Stack(
                              children: [
                                for (var i = 0; i < participants.length; i++)
                                  PositionedDirectional(
                                    start: i * Grid.twelve,
                                    child: SmallAvatar(
                                      pubkey: participants[i],
                                      userCache: userCache,
                                    ),
                                  ),
                              ],
                            ),
                          ),
                          const SizedBox(width: Grid.half),
                        ],
                        if (label != null)
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
                        if (working.isNotEmpty) ...[
                          if (label != null) const SizedBox(width: Grid.xs),
                          const TypingDots(
                            key: ValueKey('thread-working-dots'),
                            dotSize: 5,
                          ),
                        ],
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
