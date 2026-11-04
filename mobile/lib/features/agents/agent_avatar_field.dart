import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_shape.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import 'agent_avatar_editor_page.dart';
import 'agent_avatar_hero.dart';
import 'agent_avatar_picker.dart';
import 'agent_character_artwork.dart';

/// An agent's avatar with a Choose avatar action that opens the full-screen
/// editor. [editing] is true while the editor is open or returning, so the
/// page can keep native navigation out of the avatar's flight.
class AgentAvatarField extends StatelessWidget {
  const AgentAvatarField({
    required this.selection,
    required this.heroTag,
    required this.editing,
    required this.onChanged,
    this.enabled = true,
    this.showChoose = true,
    super.key,
  });
  final AgentAvatarSelection selection;
  final Object heroTag;
  final ValueNotifier<bool> editing;
  final ValueChanged<AgentAvatarSelection> onChanged;
  final bool enabled;
  final bool showChoose;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        children: [
          Semantics(
            label: 'Agent avatar',
            image: true,
            child: AgentAvatarHero(
              tag: heroTag,
              selection: selection,
              child: TickerMode(
                key: const ValueKey('create-agent-avatar-playback'),
                enabled: !editing.value,
                child: SizedBox.square(
                  dimension: 160,
                  child: selection.bytes != null
                      ? AvatarClip(
                          isAgent: true,
                          child: Image.memory(
                            selection.bytes!,
                            fit: BoxFit.cover,
                          ),
                        )
                      : PresetAgentAvatar(
                          imageUrl: selection.url!,
                          posterPlaceholder: characterPoster(selection.url!),
                          size: 160,
                          fallback: const Icon(
                            Icons.smart_toy_outlined,
                            size: 80,
                          ),
                        ),
                ),
              ),
            ),
          ),
          if (showChoose) ...[
            const SizedBox(height: Grid.xxs),
            _ChooseAvatarButton(
              transitioning: editing.value,
              onPressed: !enabled || editing.value
                  ? null
                  : () async {
                      if (editing.value) return;
                      editing.value = true;
                      try {
                        FocusScope.of(context).unfocus();
                        if (selection.url != null) {
                          await precacheImage(
                            NetworkImage(selection.url!),
                            context,
                            onError: (_, _) {},
                          ).timeout(
                            const Duration(milliseconds: 300),
                            onTimeout: () {},
                          );
                          if (!context.mounted) return;
                        }
                        // Remove the UIKit control before the Hero
                        // crosses its bounds. Keep it absent until the
                        // reverse flight's overlay has been removed.
                        await WidgetsBinding.instance.endOfFrame;
                        if (!context.mounted) return;
                        final route = PageRouteBuilder<AgentAvatarSelection>(
                          transitionDuration:
                              MediaQuery.disableAnimationsOf(context)
                              ? Duration.zero
                              : const Duration(milliseconds: 240),
                          reverseTransitionDuration:
                              MediaQuery.disableAnimationsOf(context)
                              ? Duration.zero
                              : const Duration(milliseconds: 240),
                          pageBuilder: (_, _, _) => AgentAvatarEditorPage(
                            initialSelection: selection,
                            heroTag: heroTag,
                          ),
                          transitionsBuilder: (_, animation, _, child) {
                            final progress = CurvedAnimation(
                              parent: animation,
                              curve: Curves.fastOutSlowIn,
                              reverseCurve: Curves.fastOutSlowIn.flipped,
                            );
                            return FadeTransition(
                              opacity: progress,
                              child: child,
                            );
                          },
                        );
                        final result = await Navigator.of(
                          context,
                        ).push<AgentAvatarSelection>(route);

                        if (context.mounted && result != null) {
                          onChanged(result);
                        }
                        await route.completed;
                      } finally {
                        if (context.mounted) {
                          editing.value = false;
                        }
                      }
                    },
            ),
          ],
        ],
      ),
    );
  }
}

/// Platform capsule action for entering the dedicated avatar editor.
class _ChooseAvatarButton extends StatelessWidget {
  const _ChooseAvatarButton({
    required this.onPressed,
    required this.transitioning,
  });
  final bool transitioning;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      if (transitioning) {
        return SizedBox(
          width: 160,
          height: 48,
          child: Center(
            child: TextButton(
              onPressed: null,
              child: const Text('Choose avatar'),
            ),
          ),
        );
      }
      return IosGlassNavigationAction(
        label: 'Choose avatar',
        width: 160,
        onPressed: onPressed,
      );
    }
    return FilledButton.tonal(
      style: FilledButton.styleFrom(shape: const StadiumBorder()),
      onPressed: onPressed,
      child: const Text('Choose avatar'),
    );
  }
}
