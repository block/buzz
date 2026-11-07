import 'package:flutter/material.dart';
import '../../shared/theme/theme.dart';
import 'remote_work_cloud.dart';
import 'typing_dots.dart';

/// Pending reply with three staggered dots using the desktop typing bounce.
class AgentTypingBubble extends StatelessWidget {
  const AgentTypingBubble({super.key, this.remote = false});

  /// Whether a remote agent is working, shown by a cloud before the dots.
  final bool remote;

  @override
  Widget build(BuildContext context) {
    return Container(
      key: const ValueKey('agent-typing-bubble'),
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xs,
        vertical: Grid.xxs,
      ),
      decoration: BoxDecoration(
        color: context.colors.secondaryContainer,
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      child: SizedBox(
        height: 24,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (remote) ...[
              const RemoteWorkCloud(),
              const SizedBox(width: Grid.half),
            ],
            const TypingDots(),
          ],
        ),
      ),
    );
  }
}
