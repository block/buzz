import 'package:flutter/material.dart';

import '../../shared/relay/media_image.dart';

import 'agent_avatar_geometry.dart';

/// Compensates for the shared transparent padding in the Buzzy assets.
class AgentCharacterAlignment extends StatelessWidget {
  const AgentCharacterAlignment({
    super.key,
    required this.url,
    required this.child,
    this.scale = 1,
  });
  final String url;
  final Widget child;
  final double scale;
  @override
  Widget build(BuildContext context) => FractionalTranslation(
    translation: Offset(0, agentCharacterOffset(url, scale: scale)),
    child: child,
  );
}

/// One full-resolution cache entry shared by tray, flight, and video placeholder.
/// Layout-sized decodes would otherwise reload at every animation frame.
Widget characterPoster(String url) => MediaImage(
  url: url,
  fit: BoxFit.contain,
  boundDecodeToLayout: false,
  errorBuilder: (_, _, _) => const Icon(Icons.smart_toy_outlined),
);
