import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/emoji/emoji_data.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/emoji/emoji_search.dart';
import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/ios_native_segmented_control.dart';
import '../../shared/widgets/ios_native_skin_tone_control.dart';
import '../../shared/widgets/playing_avatar_image.dart';
import 'animated_avatar_capture.dart';
import 'avatar_background_grid.dart';
import 'avatar_editor_option_button.dart';
import 'profile_avatar_crop_page.dart';
import 'profile_avatar_draft.dart';

part 'profile_avatar_editor/emoji_avatar_picker.dart';

/// Avatar kinds shared with the desktop profile editor.
enum ProfileAvatarMode { image, emoji, animated }

/// Builds the animated capture surface for the profile avatar editor.
typedef AnimatedAvatarCaptureBuilder =
    Widget Function({
      required double height,
      required ValueChanged<Future<ProfileAvatarDraft?> Function()?>
      onPrepareChanged,
    });

const _previewSize = 220.0;
const _motionDuration = Duration(milliseconds: 240);
const _previewSquishDuration = Duration(milliseconds: 200);
const Curve _entranceCurve = Curves.easeOutCubic;
const Curve _exitCurve = Curves.easeInCubic;
const Curve _modeTransitionCurve = Cubic(0.22, 1, 0.36, 1);
const double _modeTransitionDistance = 24;
const double _previewBlockSize = 228;
const double _modeControlHeight = 40;
const double _editorControlsBottom = Grid.xl + Grid.xxs;
const double _editorRailHeight = 88;
const double _previewControlGap = Grid.twelve;
const double _emojiPickerPreviewShift = 140;
// Settings' avatar center sits 96dp below its standard app bar, including its
// lower 8dp app-bar rail. The expanded editor preview is centred on screen.
const double _settingsAvatarCenterBelowAppBar = 96;

/// In-page profile avatar editor used on Android and iOS.
class ProfileAvatarEditor extends HookConsumerWidget {
  const ProfileAvatarEditor({
    super.key,
    required this.currentAvatarUrl,
    required this.fallbackInitial,
    required this.draft,
    required this.mode,
    required this.transition,
    required this.onModeChanged,
    required this.onDraftChanged,
    required this.onAnimatedPrepareChanged,
    this.animatedCaptureBuilder,
  });

  final String? currentAvatarUrl;
  final String fallbackInitial;
  final ProfileAvatarDraft? draft;
  final ProfileAvatarMode mode;
  final Animation<double> transition;
  final ValueChanged<ProfileAvatarMode> onModeChanged;
  final ValueChanged<ProfileAvatarDraft?> onDraftChanged;
  final ValueChanged<Future<ProfileAvatarDraft?> Function()?>
  onAnimatedPrepareChanged;
  final AnimatedAvatarCaptureBuilder? animatedCaptureBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final selectedEmoji = useState('😊');
    final initialColor = useMemoized(
      () => emojiAvatarColors[Random().nextInt(18)],
      const [],
    );
    final selectedColor = useState(initialColor);
    final emojiSection = useState(_EmojiEditorSection.emoji);
    final emojiPreviewKey = useState(0);
    final isPickingImage = useState(false);
    final error = useState<String?>(null);
    final dataset = ref.watch(emojiDatasetOrEmptyProvider);
    final modeTransitionDirection = useRef(1.0);
    final modeTransitionController = useAnimationController(
      duration: _motionDuration,
      initialValue: 1,
    );
    final modeTransitionValue = useAnimation(modeTransitionController);
    final modeTransitionProgress = reduceMotion
        ? 1.0
        : _modeTransitionCurve.transform(modeTransitionValue);

    void selectMode(ProfileAvatarMode nextMode) {
      if (nextMode == mode) return;
      modeTransitionDirection.value = nextMode.index > mode.index ? 1 : -1;
      unawaited(HapticFeedback.selectionClick());
      onAnimatedPrepareChanged(null);
      if (!reduceMotion) modeTransitionController.value = 0;
      onModeChanged(nextMode);
      if (nextMode == ProfileAvatarMode.emoji) {
        onDraftChanged(
          ProfileUrlAvatarDraft(
            emojiAvatarDataUrl(selectedEmoji.value, selectedColor.value),
          ),
        );
      }
      if (reduceMotion) {
        modeTransitionController.value = 1;
      } else {
        unawaited(modeTransitionController.forward());
      }
    }

    void updateEmojiPreview() {
      emojiPreviewKey.value++;
      onDraftChanged(
        ProfileUrlAvatarDraft(
          emojiAvatarDataUrl(selectedEmoji.value, selectedColor.value),
        ),
      );
    }

    Future<void> selectImage({required bool camera}) async {
      if (isPickingImage.value) return;
      isPickingImage.value = true;
      error.value = null;
      try {
        final service = ref.read(mediaUploadServiceProvider);
        unawaited(HapticFeedback.lightImpact());
        final picked = camera
            ? await service.captureImage()
            : await service.pickGalleryImage();
        if (picked == null || !context.mounted) return;
        final preparedPhoto = await service.prepareImageBytes(picked);
        if (!context.mounted) return;
        final cropped = await Navigator.of(context).push<Uint8List>(
          MaterialPageRoute(
            builder: (_) => ProfileAvatarCropPage(
              imageBytes: Future<Uint8List?>.value(preparedPhoto),
            ),
          ),
        );
        if (cropped == null) return;
        if (context.mounted) onDraftChanged(ProfileImageAvatarDraft(cropped));
      } catch (_) {
        error.value = "We couldn't prepare that photo. Try again.";
      } finally {
        if (context.mounted) isPickingImage.value = false;
      }
    }

    final previewUrl = switch (draft) {
      ProfileUrlAvatarDraft(:final url) => url,
      _ => currentAvatarUrl,
    };
    final fixedPreviewContent = switch (mode) {
      ProfileAvatarMode.image when draft is ProfileImageAvatarDraft =>
        CircleAvatar(
          key: const ValueKey('avatar-editor-fixed-preview'),
          radius: _previewSize / 2,
          backgroundImage: MemoryImage(
            (draft as ProfileImageAvatarDraft).bytes,
          ),
        ),
      ProfileAvatarMode.image => PlayingAvatarImage(
        key: const ValueKey('avatar-editor-fixed-preview'),
        imageUrl: previewUrl,
        radius: _previewSize / 2,
        backgroundColor: context.colors.primaryContainer,
        fallback: Text(
          fallbackInitial,
          style: context.textTheme.displayLarge?.copyWith(
            color: context.colors.onPrimaryContainer,
          ),
        ),
      ),
      ProfileAvatarMode.emoji => _EmojiAvatarPreview(
        key: const ValueKey('avatar-editor-fixed-preview'),
        emoji: selectedEmoji.value,
        color: Color(selectedColor.value),
        animationKey: emojiPreviewKey.value,
        reduceMotion: reduceMotion,
      ),
      ProfileAvatarMode.animated => null,
    };
    final fixedPreview = fixedPreviewContent == null
        ? null
        : Stack(
            alignment: Alignment.center,
            children: [
              fixedPreviewContent,
              if (isPickingImage.value)
                Container(
                  key: const ValueKey('avatar-image-loading-overlay'),
                  width: _previewSize,
                  height: _previewSize,
                  decoration: BoxDecoration(
                    color: context.colors.surface.withValues(alpha: 0.72),
                    shape: BoxShape.circle,
                  ),
                  child: const Center(
                    child: BuzzLoadingIndicator(
                      semanticLabel: 'Preparing photo',
                    ),
                  ),
                ),
            ],
          );

    final curvedEntrance = CurvedAnimation(
      parent: transition,
      curve: _entranceCurve,
      // The controller runs from 1 → 0 on exit. An ease-in value curve makes
      // that visible reverse movement ease out from expanded to collapsed.
      reverseCurve: _exitCurve,
    );
    final appBarHeight = frostedAppBarHeight(context);
    final topPadding = appBarHeight + Grid.xs;

    return LayoutBuilder(
      builder: (context, constraints) {
        final viewportHeight = constraints.maxHeight;
        final basePreviewTop = viewportHeight / 2 - _previewBlockSize / 2;
        final maximumShift = max(
          0.0,
          basePreviewTop -
              (topPadding + _modeControlHeight + _previewControlGap),
        );
        final requestedShift = mode != ProfileAvatarMode.emoji
            ? 0.0
            : emojiSection.value == _EmojiEditorSection.emoji
            ? _emojiPickerPreviewShift
            : avatarBackgroundPreviewShift;
        final previewShift = min(requestedShift, maximumShift);
        final previewTop = basePreviewTop - previewShift;
        final fixedContentTop =
            previewTop + _previewBlockSize + _previewControlGap;
        final modeTop = mode == ProfileAvatarMode.animated
            ? basePreviewTop
            : fixedContentTop;
        final modeHeight = max(
          0.0,
          viewportHeight - _editorControlsBottom - modeTop,
        );
        final modeContent = switch (mode) {
          ProfileAvatarMode.image => _ImageMode(
            key: const ValueKey(0),
            height: modeHeight,
            isPicking: isPickingImage.value,
            onCamera: () => unawaited(selectImage(camera: true)),
            onLibrary: () => unawaited(selectImage(camera: false)),
          ),
          ProfileAvatarMode.emoji => _EmojiMode(
            key: const ValueKey(1),
            height: modeHeight,
            activeSection: emojiSection.value,
            dataset: dataset,
            selectedEmoji: selectedEmoji.value,
            selectedColor: selectedColor.value,
            onSectionChanged: (section) => emojiSection.value = section,
            onEmojiSelected: (emoji) {
              selectedEmoji.value = emoji;
              updateEmojiPreview();
            },
            onColorSelected: (color) {
              selectedColor.value = color;
              updateEmojiPreview();
            },
          ),
          ProfileAvatarMode.animated => KeyedSubtree(
            key: const ValueKey(2),
            child:
                animatedCaptureBuilder?.call(
                  height: modeHeight,
                  onPrepareChanged: onAnimatedPrepareChanged,
                ) ??
                AnimatedAvatarCapture(
                  height: modeHeight,
                  onPrepareChanged: onAnimatedPrepareChanged,
                ),
          ),
        };
        final collapsedPreviewOffset =
            appBarHeight +
            _settingsAvatarCenterBelowAppBar -
            (previewTop + _previewBlockSize / 2);
        final transitionedModeContent = mode == ProfileAvatarMode.animated
            ? modeContent
            : ClipRect(
                child: Transform.translate(
                  key: const ValueKey('avatar-mode-transition-transform'),
                  offset: Offset(
                    modeTransitionDirection.value *
                        _modeTransitionDistance *
                        (1 - modeTransitionProgress),
                    0,
                  ),
                  child: Opacity(
                    key: const ValueKey('avatar-mode-transition-opacity'),
                    opacity: modeTransitionProgress,
                    child: modeContent,
                  ),
                ),
              );

        return Stack(
          key: const ValueKey('avatar-editor-content'),
          clipBehavior: Clip.none,
          children: [
            Positioned(
              left: Grid.gutter,
              right: Grid.gutter,
              top: topPadding,
              child: FadeTransition(
                opacity: curvedEntrance,
                child: ScaleTransition(
                  scale: Tween(begin: 0.96, end: 1.0).animate(curvedEntrance),
                  child: Builder(
                    builder: (context) {
                      if (defaultTargetPlatform == TargetPlatform.iOS) {
                        return IosNativeSegmentedControl(
                          key: const ValueKey('avatar-mode-control'),
                          items: const ['Image', 'Emoji', 'Animated'],
                          selectedIndex: mode.index,
                          onChanged: (index) {
                            if (index < 0 ||
                                index >= ProfileAvatarMode.values.length) {
                              return;
                            }
                            selectMode(ProfileAvatarMode.values[index]);
                          },
                        );
                      }
                      return _AvatarModeControl(
                        selected: mode,
                        reduceMotion: reduceMotion,
                        onSelected: selectMode,
                      );
                    },
                  ),
                ),
              ),
            ),
            if (fixedPreview != null)
              AnimatedPositioned(
                key: const ValueKey('avatar-preview-position'),
                duration: reduceMotion
                    ? Duration.zero
                    : const Duration(milliseconds: 150),
                curve: Curves.easeOutCubic,
                left: Grid.gutter,
                right: Grid.gutter,
                top: previewTop,
                height: _previewBlockSize,
                child: Center(
                  child: AnimatedBuilder(
                    animation: curvedEntrance,
                    child: fixedPreview,
                    builder: (context, child) {
                      final progress = curvedEntrance.value;
                      return Transform.translate(
                        key: const ValueKey('avatar-editor-entrance-transform'),
                        offset: Offset(
                          0,
                          collapsedPreviewOffset * (1 - progress),
                        ),
                        child: Transform.scale(
                          scale:
                              128 / _previewSize +
                              (1 - 128 / _previewSize) * progress,
                          child: child,
                        ),
                      );
                    },
                  ),
                ),
              ),
            AnimatedPositioned(
              duration: reduceMotion
                  ? Duration.zero
                  : const Duration(milliseconds: 150),
              curve: Curves.easeOutCubic,
              left: Grid.gutter,
              right: Grid.gutter,
              top: modeTop,
              height: modeHeight,
              child: FadeTransition(
                opacity: curvedEntrance,
                child: transitionedModeContent,
              ),
            ),
            if (error.value != null)
              Positioned(
                left: Grid.gutter,
                right: Grid.gutter,
                bottom: _editorControlsBottom + _editorRailHeight,
                child: Text(
                  error.value!,
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}

class _AvatarModeControl extends StatelessWidget {
  const _AvatarModeControl({
    required this.selected,
    required this.reduceMotion,
    required this.onSelected,
  });

  final ProfileAvatarMode selected;
  final bool reduceMotion;
  final ValueChanged<ProfileAvatarMode> onSelected;

  @override
  Widget build(BuildContext context) => Material(
    key: const ValueKey('avatar-mode-control'),
    color: context.colors.surfaceContainerHighest,
    borderRadius: BorderRadius.circular(Radii.full),
    clipBehavior: Clip.antiAlias,
    child: Padding(
      padding: const EdgeInsets.all(Grid.quarter),
      child: LayoutBuilder(
        builder: (context, constraints) {
          final segmentWidth =
              constraints.maxWidth / ProfileAvatarMode.values.length;
          return Stack(
            children: [
              TweenAnimationBuilder<double>(
                tween: Tween(end: selected.index.toDouble()),
                duration: reduceMotion ? Duration.zero : _motionDuration,
                curve: Curves.easeInOut,
                builder: (context, position, child) => Transform.translate(
                  offset: Offset(segmentWidth * position, 0),
                  child: child,
                ),
                child: SizedBox(
                  width: segmentWidth,
                  height: 36,
                  child: DecoratedBox(
                    key: const ValueKey('avatar-mode-indicator'),
                    decoration: BoxDecoration(
                      color: context.colors.surface,
                      borderRadius: BorderRadius.circular(Radii.full),
                      boxShadow: [
                        BoxShadow(
                          color: Colors.black.withValues(alpha: 0.08),
                          blurRadius: 4,
                          offset: const Offset(0, 1),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              Row(
                children: [
                  for (final mode in ProfileAvatarMode.values)
                    Expanded(
                      child: InkWell(
                        key: ValueKey('avatar-mode-${mode.name}'),
                        borderRadius: BorderRadius.circular(Radii.full),
                        onTap: () => onSelected(mode),
                        child: SizedBox(
                          height: 36,
                          child: Center(
                            child: Text(
                              switch (mode) {
                                ProfileAvatarMode.image => 'Image',
                                ProfileAvatarMode.emoji => 'Emoji',
                                ProfileAvatarMode.animated => 'Animated',
                              },
                              style: context.textTheme.labelLarge?.copyWith(
                                fontWeight: mode == selected
                                    ? FontWeight.w600
                                    : FontWeight.w500,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            ],
          );
        },
      ),
    ),
  );
}

class _ImageMode extends StatelessWidget {
  const _ImageMode({
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
              child: AvatarEditorOptionButton(
                key: const ValueKey('image-source-camera'),
                icon: LucideIcons.camera,
                label: 'Camera',
                selected: false,
                onTap: isPicking ? null : onCamera,
                labelMaxWidth: 96,
              ),
            ),
            const SizedBox(width: Grid.half),
            Expanded(
              child: AvatarEditorOptionButton(
                key: const ValueKey('image-source-library'),
                icon: LucideIcons.images,
                label: 'Photo Library',
                selected: false,
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
