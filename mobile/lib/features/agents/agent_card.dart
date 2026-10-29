import 'dart:ui';

import 'package:flutter/material.dart';

import '../../shared/animated_avatar.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/progressive_animated_avatar.dart';

/// A directory card using the desktop's portrait and faded-artwork treatment.
class AgentCard extends StatelessWidget {
  const AgentCard({
    required this.name,
    required this.onOpen,
    this.imageUrl,
    this.location,
    this.footer,
    this.subtitle,
    super.key,
  });

  final String name;
  final String? subtitle;
  final String? imageUrl;

  /// Only supplied when the host is known; directory membership is not proof.
  final String? location;
  final VoidCallback onOpen;

  /// Optional host-owned controls. The card does not invent edit capability.
  final Widget? footer;

  @override
  Widget build(BuildContext context) {
    final animation = parseAnimatedAvatarUrl(imageUrl);
    final poster = animation?.posterUrl ?? imageUrl;
    final fallback = Center(
      child: Text(
        name.characters.first,
        style: context.textTheme.headlineMedium,
      ),
    );
    final portrait =
        animation != null &&
            !MediaQuery.disableAnimationsOf(context) &&
            TickerMode.valuesOf(context).enabled
        ? ProgressiveAnimatedAvatar(
            descriptor: animation,
            fallback: fallback,
            fit: BoxFit.contain,
          )
        : AvatarImageContent(
            imageUrl: poster,
            fallback: fallback,
            fit: BoxFit.contain,
          );
    return ClipRRect(
      borderRadius: BorderRadius.circular(Radii.card),
      child: ColoredBox(
        color: context.colors.surfaceContainerLow,
        child: Stack(
          fit: StackFit.expand,
          children: [
            if (poster != null && !MediaQuery.highContrastOf(context))
              Positioned.fill(
                child: ExcludeSemantics(
                  child: RepaintBoundary(
                    child: ShaderMask(
                      blendMode: BlendMode.dstIn,
                      shaderCallback: (bounds) => const LinearGradient(
                        begin: Alignment.topCenter,
                        end: Alignment.bottomCenter,
                        colors: [Colors.white, Colors.transparent],
                        stops: [0.05, 0.75],
                      ).createShader(bounds),
                      child: Opacity(
                        opacity: 0.55,
                        child: ImageFiltered(
                          imageFilter: ImageFilter.blur(sigmaX: 24, sigmaY: 24),
                          child: Transform.scale(
                            scale: 1.8,
                            child: AvatarImageContent(
                              imageUrl: poster,
                              fallback: const SizedBox.shrink(),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            Material(
              color: Colors.transparent,
              child: InkWell(
                onTap: onOpen,
                excludeFromSemantics: true,
                child: Padding(
                  padding: const EdgeInsets.all(Grid.twelve),
                  child: Column(
                    children: [
                      Expanded(
                        child: Semantics(
                          container: true,
                          button: true,
                          label: subtitle == null
                              ? 'Open $name'
                              : 'Open $name, $subtitle',
                          onTap: onOpen,
                          child: ExcludeSemantics(
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              children: [
                                Expanded(
                                  child: Center(
                                    child: RepaintBoundary(
                                      child: SizedBox.square(
                                        dimension: 104,
                                        child: ClipRRect(
                                          borderRadius: BorderRadius.circular(
                                            Radii.card,
                                          ),
                                          child: portrait,
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                                Text(
                                  name,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  textAlign: TextAlign.center,
                                  style: context.textTheme.titleSmall?.copyWith(
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                      const SizedBox(height: Grid.half),
                      SizedBox(
                        height:
                            44 *
                            (MediaQuery.textScalerOf(context).scale(16) / 16)
                                .clamp(1, double.infinity),
                        child:
                            footer ??
                            Center(
                              child: ExcludeSemantics(
                                child: Text(
                                  subtitle ?? '',
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  textAlign: TextAlign.center,
                                  style: context.textTheme.bodySmall?.copyWith(
                                    color: context.colors.onSurfaceVariant,
                                  ),
                                ),
                              ),
                            ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
