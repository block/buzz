import 'package:flutter/material.dart';
import '../theme/theme.dart';
import '../theme/buzz_icons.dart';
import 'ios_glass_navigation_button.dart';
import 'avatar_editor_option_button.dart';

/// Camera and photo-library actions shared by avatar editors.
class AvatarImageSourceControls extends StatelessWidget {
  const AvatarImageSourceControls({
    super.key,
    required this.height,
    required this.isPicking,
    required this.onCamera,
    required this.onLibrary,
  });

  final double height;
  final bool isPicking;
  final VoidCallback onCamera;
  final VoidCallback onLibrary;

  @override
  Widget build(BuildContext context) => SizedBox(
    height: height,
    child: Column(
      children: [
        const Spacer(),
        Row(
          children: [
            const Spacer(),
            Expanded(
              child: _ImageSourceOption(
                key: const ValueKey('image-source-camera'),
                icon: BuzzIcons.camera,
                iosIcon: IosGlassNavigationIcon.camera,
                label: 'Camera',
                onTap: isPicking ? null : onCamera,
                labelMaxWidth: 96,
              ),
            ),
            const SizedBox(width: Grid.half),
            Expanded(
              child: _ImageSourceOption(
                key: const ValueKey('image-source-library'),
                icon: BuzzIcons.images,
                iosIcon: IosGlassNavigationIcon.photoLibrary,
                label: 'Photo Library',
                onTap: isPicking ? null : onLibrary,
                labelMaxWidth: 104,
              ),
            ),
            const Spacer(),
          ],
        ),
      ],
    ),
  );
}

class _ImageSourceOption extends StatelessWidget {
  const _ImageSourceOption({
    super.key,
    required this.icon,
    required this.iosIcon,
    required this.label,
    required this.onTap,
    required this.labelMaxWidth,
  });

  final IconData icon;
  final IosGlassNavigationIcon iosIcon;
  final String label;
  final VoidCallback? onTap;
  final double labelMaxWidth;

  @override
  Widget build(BuildContext context) => AvatarEditorOptionButton(
    icon: icon,
    iosIcon: iosIcon,
    label: label,
    selected: false,
    onTap: onTap,
    labelMaxWidth: labelMaxWidth,
  );
}
