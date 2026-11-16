import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../../shared/relay/relay.dart';

import 'agent_avatar_picker.dart';

/// A soft artwork tint behind the avatar editor, matching desktop creation.
class AgentAvatarBackdrop extends StatelessWidget {
  const AgentAvatarBackdrop({super.key, required this.selection});

  final AgentAvatarSelection selection;

  @override
  Widget build(BuildContext context) {
    if (MediaQuery.highContrastOf(context)) return const SizedBox.expand();
    return IgnorePointer(
      child: ExcludeSemantics(
        child: ClipRect(
          child: ShaderMask(
            blendMode: BlendMode.dstIn,
            shaderCallback: (bounds) => const LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              // Keep the wash near the portrait; the lower controls sit clear.
              colors: [Colors.white, Color(0x66FFFFFF), Colors.transparent],
              stops: [0, 0.25, 0.68],
            ).createShader(bounds),
            child: AnimatedSwitcher(
              duration: MediaQuery.disableAnimationsOf(context)
                  ? Duration.zero
                  : const Duration(milliseconds: 240),
              layoutBuilder: (current, previous) => Stack(
                fit: StackFit.expand,
                children: [...previous, ?current],
              ),
              child: RepaintBoundary(
                key: ValueKey((
                  selection.url,
                  selection.bytes,
                  selection.color,
                )),
                child: Opacity(
                  opacity: selection.emoji != null ? 0.14 : 0.64,
                  child: ImageFiltered(
                    imageFilter: ui.ImageFilter.blur(sigmaX: 40, sigmaY: 40),
                    child: Transform.scale(
                      // Crop into the artwork so it reads as abstract color.
                      scale: 3.5,
                      alignment: const Alignment(0, 0.12),
                      child: selection.emoji != null
                          ? ColoredBox(color: Color(selection.color!))
                          : ColorFiltered(
                              // The desktop backdrop uses saturate(4).
                              colorFilter: const ColorFilter.matrix([
                                3.3622,
                                -2.1456,
                                -0.2166,
                                0,
                                0,
                                -0.6378,
                                1.8544,
                                -0.2166,
                                0,
                                0,
                                -0.6378,
                                -2.1456,
                                3.7834,
                                0,
                                0,
                                0,
                                0,
                                0,
                                1,
                                0,
                              ]),
                              child: selection.bytes != null
                                  ? Image.memory(
                                      selection.bytes!,
                                      fit: BoxFit.cover,
                                      cacheWidth: 256,
                                      excludeFromSemantics: true,
                                      errorBuilder: (_, _, _) =>
                                          const SizedBox.expand(),
                                    )
                                  : MediaImage(
                                      url: selection.url!,
                                      fit: BoxFit.cover,
                                      // About 256 physical pixels: the
                                      // blur hides any more detail.
                                      decodeWidth: 96,
                                      errorBuilder: (_, _, _) =>
                                          const SizedBox.expand(),
                                    ),
                            ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
