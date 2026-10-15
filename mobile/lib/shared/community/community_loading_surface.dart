import 'package:flutter/material.dart';
import '../theme/theme.dart';
import '../widgets/buzz_loading_indicator.dart';
import 'community_avatar.dart';

/// Holds the incoming community at the switch animation's centered position.
class CommunityLoadingSurface extends StatelessWidget {
  const CommunityLoadingSurface({super.key, this.name, this.relayUrl});

  final String? name;
  final String? relayUrl;

  @override
  Widget build(BuildContext context) => ColoredBox(
    color: context.colors.surface,
    child: Stack(
      fit: StackFit.expand,
      children: [
        if (relayUrl != null)
          Center(
            child: CommunityAvatar(name: name, relayUrl: relayUrl, size: 132),
          ),
        Positioned(
          top: MediaQuery.sizeOf(context).height / 2 + 66 + Grid.md,
          left: 0,
          right: 0,
          child: Center(
            child: BuzzLoadingIndicator(
              size: 18,
              color: context.colors.onSecondaryContainer,
              semanticLabel: name == null ? 'Connecting' : 'Loading $name',
            ),
          ),
        ),
      ],
    ),
  );
}
