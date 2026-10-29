import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

/// The desktop design system's soft, wide robot mark.
class BuzzAgentIcon extends StatelessWidget {
  const BuzzAgentIcon({super.key, required this.size, required this.color});

  final double size;
  final Color color;

  @override
  Widget build(BuildContext context) => SvgPicture.string(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" '
    'fill="none" stroke="currentColor" stroke-width="2" '
    'stroke-linecap="round" stroke-linejoin="round">'
    '<rect x="4.75" y="7" width="14.5" height="13.25" rx="4.5"/>'
    '<path d="M12 7V2.5M3.25 11.5v4.25M20.75 11.5v4.25"/>'
    '<g fill="currentColor" stroke="none">'
    '<circle cx="9" cy="13.25" r="1.4"/>'
    '<circle cx="15" cy="13.25" r="1.4"/>'
    '</g></svg>',
    width: size,
    height: size,
    colorFilter: ColorFilter.mode(color, BlendMode.srcIn),
    excludeFromSemantics: true,
  );
}
