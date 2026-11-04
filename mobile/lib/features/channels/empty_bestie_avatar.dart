import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/widgets/agent_navigation_icon.dart';
import '../../shared/widgets/avatar_image.dart';

final _characters = [
  for (var i = 0; i < AgentAvatarPack.gloopies.indices.length; i++)
    for (final pack in [
      AgentAvatarPack.fuzzies,
      AgentAvatarPack.gloopies,
      AgentAvatarPack.figgies,
    ])
      if (i < pack.indices.length) agentAvatarCatalog[pack.indices[i]],
];

/// A quiet invitation to create a first companion, cycling every five seconds.
class EmptyBestieAvatar extends HookWidget {
  const EmptyBestieAvatar({super.key});

  @override
  Widget build(BuildContext context) {
    final index = useState(0);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final lifecycle = useAppLifecycleState();
    final visible =
        TickerMode.valuesOf(context).enabled &&
        ModalRoute.of(context)?.isCurrent != false &&
        (lifecycle == null || lifecycle == AppLifecycleState.resumed);
    useEffect(() {
      if (reducedMotion || !visible) return null;
      final timer = Timer.periodic(const Duration(seconds: 5), (_) {
        index.value = (index.value + 1) % _characters.length;
      });
      return timer.cancel;
    }, [reducedMotion, visible]);
    final character = _characters[index.value];
    useEffect(() {
      var cancelled = false;
      if (visible && !reducedMotion) {
        final nextPoster =
            _characters[(index.value + 1) % _characters.length].url;
        // precacheImage reads inherited configuration; do so after hook setup.
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (cancelled || !context.mounted) return;
          unawaited(
            precacheImage(
              NetworkImage(nextPoster),
              context,
              onError: (_, _) {}, // Keep the offline avatar fallback.
            ),
          );
        });
      }
      return () => cancelled = true;
    }, [index.value, visible, reducedMotion]);
    return AnimatedSwitcher(
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 280),
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      transitionBuilder: (child, animation) =>
          ScaleTransition(scale: animation, child: child),
      child: SizedBox.square(
        key: ValueKey(character.url),
        dimension: 47.88,
        child: AvatarImageContent(
          imageUrl: character.url,
          preferThumbnail: true,
          fit: BoxFit.contain,
          fallback: const AgentNavigationIcon(size: 24),
        ),
      ),
    );
  }
}
