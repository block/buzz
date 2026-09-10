import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Maximum width for focused utility pages such as Settings and Profile.
const double centeredUtilityPageMaxWidth = 640;

/// Keeps focused utility-page chrome and content together on a full-width
/// utility canvas.
class CenteredUtilityPage extends StatelessWidget {
  const CenteredUtilityPage({required this.child, this.contentKey, super.key});

  final Widget child;
  final Key? contentKey;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      key: const ValueKey('centered-utility-page-background'),
      color: context.colors.surfaceContainerHighest,
      child: Align(
        alignment: Alignment.topCenter,
        child: ConstrainedBox(
          key: contentKey,
          constraints: const BoxConstraints(
            maxWidth: centeredUtilityPageMaxWidth,
          ),
          child: child,
        ),
      ),
    );
  }
}
