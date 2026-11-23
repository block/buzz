import 'package:flutter/material.dart';
import '../../shared/theme/theme.dart';
import 'typing_dots.dart';

/// Pending reply with three staggered dots using the desktop typing bounce.
class AgentTypingBubble extends StatelessWidget {
  const AgentTypingBubble({super.key});

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
      child: const SizedBox(height: 24, child: TypingDots()),
    );
  }
}
