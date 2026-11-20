import 'package:flutter/material.dart';

import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/theme/theme.dart';

class SectionOptionsButton extends StatelessWidget {
  const SectionOptionsButton({
    super.key,
    required this.tooltip,
    required this.onPressed,
  });

  final String tooltip;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: tooltip,
    onTap: onPressed,
    excludeSemantics: true,
    child: Tooltip(
      message: tooltip,
      child: SizedBox(
        width: 64,
        height: 48,
        child: InkWell(
          onTap: onPressed,
          borderRadius: BorderRadius.circular(Radii.full),
          child: Align(
            alignment: Alignment.centerRight,
            child: SizedBox(
              width: 44,
              child: Center(
                child: Icon(
                  BuzzIcons.ellipsis,
                  size: 14,
                  color: navigationAccessoryForeground(context),
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
}
