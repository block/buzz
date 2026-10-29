import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

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
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/ios_native_skin_tone_control.dart';
import '../../shared/widgets/playing_avatar_image.dart';
import 'animated_avatar_capture.dart';
import 'camera_disposal_barrier.dart';
import 'avatar_background_grid.dart';
import 'avatar_editor_option_button.dart';
import 'emoji_avatar_tile.dart';
import 'image_avatar_capture.dart';
import 'profile_avatar_crop_page.dart';
import 'profile_avatar_draft.dart';

part 'profile_avatar_editor/emoji_avatar_picker.dart';

/// Avatar kinds shared with the desktop profile editor.
enum ProfileAvatarMode {
  /// A still image selected from the camera or photo library.
  image,

  /// A system emoji composited over a selected background color.
  emoji,

  /// A short camera animation with framing and background controls.
  animated,
}

/// Builds the animated capture surface for the profile avatar editor.
typedef AnimatedAvatarCaptureBuilder =
    Widget Function({
      required double height,
      required ValueChanged<Future<ProfileAvatarDraft?> Function()?>
      onPrepareChanged,
    });

const _previewSize = 220.0;
const _motionDuration = Duration(milliseconds: 150);
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
  /// Creates an avatar editor backed by the current profile and draft state.
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
    required this.onImageCameraActiveChanged,
    this.animatedCaptureBuilder,
    this.imageCaptureBuilder,
    this.previewKey,
    this.previewOpacity = 1,
    this.emptyPreview,
    this.returningToEmpty = false,
    this.headerGap = Grid.xs,
    this.anchoredPreview,
  });

  /// The avatar URL shown until the user selects a new draft.
  final String? currentAvatarUrl;

  final GlobalKey? previewKey;
  final double previewOpacity;
  final Widget? emptyPreview;

  /// Crossfades a discarded setup draft to its original empty placeholder.
  final bool returningToEmpty;
  final double headerGap;

  /// Keeps the onboarding avatar in place while its controls appear.
  final Rect? anchoredPreview;

  /// The text initial used when [currentAvatarUrl] has no displayable image.
  final String fallbackInitial;

  /// The unsaved avatar selection for the active editing session.
  final ProfileAvatarDraft? draft;

  /// The currently selected avatar editing mode.
  final ProfileAvatarMode mode;

  /// Drives the shared preview transition into and out of editing.
  final Animation<double> transition;

  /// Called when the user selects a different avatar editing mode.
  final ValueChanged<ProfileAvatarMode> onModeChanged;

  /// Called whenever the unsaved avatar selection changes.
  final ValueChanged<ProfileAvatarDraft?> onDraftChanged;

  /// Supplies or clears the deferred animated-avatar preparation callback.
  final ValueChanged<Future<ProfileAvatarDraft?> Function()?>
  onAnimatedPrepareChanged;

  /// Reports whether the inline still camera currently owns the image editor.
  final ValueChanged<bool> onImageCameraActiveChanged;

  /// Overrides the animated capture surface, primarily for tests.
  final AnimatedAvatarCaptureBuilder? animatedCaptureBuilder;

  /// Overrides the still-image capture surface, primarily for tests.
  final ImageAvatarCaptureBuilder? imageCaptureBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sourcePreview = this.anchoredPreview;
    final initialPreviewScale =
        (sourcePreview?.width ?? _previewSize) / _previewSize;
    // Keep the editor layout full-sized while the empty native preview grows.
    final anchoredPreview = sourcePreview == null
        ? null
        : Rect.fromCenter(
            center: sourcePreview.center,
            width: _previewSize,
            height: _previewSize,
          );
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final previousAnchor = useRef(anchoredPreview);
    final anchorMoved = previousAnchor.value != anchoredPreview;
    previousAnchor.value = anchoredPreview;
    // Seed Emoji controls from the opening draft without making it the Image baseline.
    final currentEmojiAvatar = useMemoized(
      () => parseEmojiAvatarDataUrl(
        draft is ProfileUrlAvatarDraft
            ? (draft as ProfileUrlAvatarDraft).url
            : currentAvatarUrl,
      ),
      const [],
    );
    final selectedEmoji = useState(currentEmojiAvatar?.emoji ?? '😊');
    final initialColor = useMemoized(
      () =>
          currentEmojiAvatar?.colorValue ??
          emojiAvatarColors[Random().nextInt(18)],
      [currentEmojiAvatar],
    );
    final selectedColor = useState(initialColor);
    final emojiSection = useState(_EmojiEditorSection.emoji);
    final emojiPreviewKey = useState(0);
    final isPickingImage = useState(false);
    final isCapturingImage = useState(false);
    final cameraDisposal = useRef(CameraDisposalBarrier());
    final imageSelectionGeneration = useRef(0);
    final currentMode = useRef(mode)..value = mode;
    final error = useState<String?>(null);
    final dataset = ref.watch(emojiDatasetOrEmptyProvider);
    final modeTransitionDirection = useRef(1.0);
    final modeTransitionFrom = useRef(mode);
    final retainedPreview = useRef<Widget?>(null);
    final retainedPreviewTop = useRef(0.0);
    final currentPreviewTop = useRef<double?>(null);
    final modeStartPreviewTop = useRef<double?>(null);
    final modeTransitionController = useAnimationController(
      duration: anchoredPreview == null
          ? _motionDuration
          : const Duration(milliseconds: 200),
      initialValue: 1,
    );
    final modeTransitionValue = useAnimation(modeTransitionController);
    final modeTransitionProgress = reduceMotion
        ? 1.0
        : _modeTransitionCurve.transform(modeTransitionValue);

    void selectMode(ProfileAvatarMode nextMode) {
      if (nextMode == mode) return;
      modeStartPreviewTop.value = currentPreviewTop.value;
      modeTransitionFrom.value = mode;
      modeTransitionDirection.value = nextMode.index > mode.index ? 1 : -1;
      unawaited(HapticFeedback.selectionClick());
      onAnimatedPrepareChanged(null);
      if (mode == ProfileAvatarMode.image) {
        imageSelectionGeneration.value++;
        isPickingImage.value = false;
        isCapturingImage.value = false;
        onImageCameraActiveChanged(false);
      }
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

    Future<void> selectGalleryImage() async {
      if (isPickingImage.value) return;
      final operation = ++imageSelectionGeneration.value;
      bool isCurrentOperation() =>
          context.mounted &&
          currentMode.value == ProfileAvatarMode.image &&
          imageSelectionGeneration.value == operation;
      isPickingImage.value = true;
      error.value = null;
      try {
        final service = ref.read(mediaUploadServiceProvider);
        unawaited(HapticFeedback.lightImpact());
        final picked = await service.pickGalleryImage();
        if (picked == null || !isCurrentOperation()) return;
        final preparedPhoto = await service.prepareImageBytes(picked);
        if (!context.mounted || !isCurrentOperation()) return;
        final cropped = await Navigator.of(context).push<Uint8List>(
          MaterialPageRoute(
            fullscreenDialog: true,
            builder: (_) => ProfileAvatarCropPage(
              imageBytes: Future<Uint8List?>.value(preparedPhoto),
            ),
          ),
        );
        if (cropped == null || !isCurrentOperation()) return;
        onDraftChanged(ProfileImageAvatarDraft(cropped));
      } catch (_) {
        if (isCurrentOperation()) {
          error.value = "We couldn't prepare that photo. Try again.";
        }
      } finally {
        if (isCurrentOperation()) isPickingImage.value = false;
      }
    }

    void closeImageCamera() {
      isCapturingImage.value = false;
      onImageCameraActiveChanged(false);
    }

    void acceptCameraImage(Uint8List bytes) {
      onDraftChanged(ProfileImageAvatarDraft(bytes));
      closeImageCamera();
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
      ProfileAvatarMode.image when previewUrl == null && emptyPreview != null =>
        SizedBox.square(dimension: _previewSize, child: emptyPreview),
      ProfileAvatarMode.image => PlayingAvatarImage(
        key: const ValueKey('avatar-editor-fixed-preview'),
        imageUrl: previewUrl,
        radius: _previewSize / 2,
        backgroundColor: context.controlSurface,
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
        reduceMotion:
            reduceMotion ||
            (modeTransitionFrom.value == ProfileAvatarMode.animated &&
                modeTransitionProgress < 1),
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
    if (mode != ProfileAvatarMode.animated && fixedPreview != null) {
      retainedPreview.value = mode == ProfileAvatarMode.emoji
          ? _EmojiAvatarPreview(
              emoji: selectedEmoji.value,
              color: Color(selectedColor.value),
              animationKey: emojiPreviewKey.value,
              reduceMotion: true,
            )
          : fixedPreview;
    }

    final curvedEntrance = CurvedAnimation(
      parent: transition,
      curve: _entranceCurve,
      // The controller runs from 1 → 0 on exit. An ease-in value curve makes
      // that visible reverse movement ease out from expanded to collapsed.
      reverseCurve: _exitCurve,
    );
    final appBarHeight = frostedAppBarHeight(context);
    final topPadding = anchoredPreview == null ? appBarHeight + headerGap : 0.0;
    // UIKit already ends the embedded editor at the bottom safe area.
    final controlsBottom = anchoredPreview == null
        ? _editorControlsBottom
        : Grid.xxs;

    return LayoutBuilder(
      builder: (context, constraints) {
        final viewportHeight = constraints.maxHeight;
        final basePreviewTop =
            anchoredPreview?.top ??
            (viewportHeight / 2 - _previewBlockSize / 2);
        final maximumShift = max(
          0.0,
          basePreviewTop -
              (topPadding + _modeControlHeight + _previewControlGap),
        );
        final requestedShift = switch (mode) {
          ProfileAvatarMode.emoji => _emojiPickerPreviewShift,
          ProfileAvatarMode.animated when anchoredPreview != null => 64.0,
          _ => 0.0,
        };
        final previewShift = min(requestedShift, maximumShift);
        final previewTop = basePreviewTop - previewShift;
        final returningToEmoji =
            anchoredPreview == null &&
            mode == ProfileAvatarMode.emoji &&
            modeTransitionFrom.value == ProfileAvatarMode.animated;
        final animatedModeHeight = max(
          0.0,
          viewportHeight - controlsBottom - basePreviewTop,
        );
        final animatedPreviewSize = animatedModeHeight < 400 ? 180.0 : 228.0;
        final animatedPreviewTop =
            basePreviewTop + (animatedPreviewSize - _previewBlockSize) / 2;
        final fixedContentTop =
            previewTop +
            (anchoredPreview?.height ?? _previewBlockSize) +
            _previewControlGap;
        final imageCameraTop =
            basePreviewTop +
            _previewBlockSize / 2 -
            imageAvatarCameraPreviewSize / 2;
        final modeTop = mode == ProfileAvatarMode.animated
            ? (anchoredPreview == null ? basePreviewTop : previewTop)
            : mode == ProfileAvatarMode.image && isCapturingImage.value
            ? imageCameraTop
            : fixedContentTop;
        // Keep the picker fixed and balance the whole preview circle between
        // the mode selector and the search row in the embedded setup editor.
        final centeredEmojiTop =
            (topPadding +
                _modeControlHeight +
                modeTop -
                (anchoredPreview?.height ?? _previewBlockSize)) /
            2;
        final targetPreviewTop =
            anchoredPreview != null && mode == ProfileAvatarMode.emoji
            ? centeredEmojiTop
            : returningToEmoji
            ? animatedPreviewTop +
                  (previewTop - animatedPreviewTop) * modeTransitionProgress
            : previewTop;
        final displayedPreviewTop = anchoredPreview != null
            ? (modeStartPreviewTop.value ?? targetPreviewTop) +
                  (targetPreviewTop -
                          (modeStartPreviewTop.value ?? targetPreviewTop)) *
                      modeTransitionProgress
            : targetPreviewTop;
        currentPreviewTop.value = displayedPreviewTop;
        if (mode != ProfileAvatarMode.animated) {
          retainedPreviewTop.value = displayedPreviewTop;
        }
        final modeHeight = max(0.0, viewportHeight - controlsBottom - modeTop);
        final contentHeight =
            anchoredPreview != null && mode == ProfileAvatarMode.emoji
            ? max(240.0, modeHeight)
            : modeHeight;
        final modeContent = switch (mode) {
          ProfileAvatarMode.image when isCapturingImage.value => KeyedSubtree(
            key: const ValueKey('image-camera-mode'),
            child:
                imageCaptureBuilder?.call(
                  height: contentHeight,
                  onAccepted: acceptCameraImage,
                  onClosed: closeImageCamera,
                ) ??
                ImageAvatarCapture(
                  height: contentHeight,
                  initialPreview: fixedPreview,
                  onAccepted: acceptCameraImage,
                  onClosed: closeImageCamera,
                  disposalBarrier: cameraDisposal.value,
                ),
          ),
          ProfileAvatarMode.image => _ImageMode(
            key: const ValueKey(0),
            height: contentHeight,
            isPicking: isPickingImage.value,
            onCamera: () {
              error.value = null;
              isCapturingImage.value = true;
              onImageCameraActiveChanged(true);
            },
            onLibrary: () => unawaited(selectGalleryImage()),
          ),
          ProfileAvatarMode.emoji => _EmojiMode(
            key: const ValueKey(1),
            height: contentHeight,
            activeSection: emojiSection.value,
            dataset: dataset,
            selectedEmoji: selectedEmoji.value,
            selectedColor: selectedColor.value,
            transitionProgress: modeTransitionProgress,
            transitionDirection: modeTransitionDirection.value,
            controlsFromBelow: anchoredPreview != null,
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
                  height: contentHeight,
                  onPrepareChanged: onAnimatedPrepareChanged,
                ) ??
                AnimatedAvatarCapture(
                  previewKey: previewKey,
                  previewSize: anchoredPreview?.width,
                  previewOffset: anchoredPreview == null
                      ? 0
                      : displayedPreviewTop - modeTop,
                  controlsTransitionProgress: anchoredPreview == null
                      ? 1
                      : modeTransitionProgress,
                  height: contentHeight,
                  onPrepareChanged: onAnimatedPrepareChanged,
                  disposalBarrier: cameraDisposal.value,
                ),
          ),
        };
        final collapsedPreviewOffset =
            appBarHeight +
            _settingsAvatarCenterBelowAppBar -
            (previewTop + _previewBlockSize / 2);
        final transitionedModeContent =
            mode == ProfileAvatarMode.animated ||
                mode == ProfileAvatarMode.emoji
            ? modeContent
            : ClipRect(
                child: Transform.translate(
                  key: const ValueKey('avatar-mode-transition-transform'),
                  offset: Offset(
                    anchoredPreview == null
                        ? modeTransitionDirection.value *
                              _modeTransitionDistance *
                              (1 - modeTransitionProgress)
                        : 0,
                    anchoredPreview == null
                        ? 0
                        : 16 * (1 - modeTransitionProgress),
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
                child: AnimatedBuilder(
                  animation: curvedEntrance,
                  builder: (context, child) => anchoredPreview == null
                      ? Transform.scale(
                          scale: 0.96 + 0.04 * curvedEntrance.value,
                          child: child,
                        )
                      : Transform.translate(
                          offset: Offset(
                            0,
                            reduceMotion ? 0 : 24 * (1 - curvedEntrance.value),
                          ),
                          child: child,
                        ),
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
            if ((fixedPreview != null || returningToEmpty) &&
                !isCapturingImage.value)
              AnimatedPositioned(
                key: const ValueKey('avatar-preview-position'),
                curve: Curves.easeOutCubic,
                left: Grid.gutter,
                right: Grid.gutter,
                top: displayedPreviewTop,
                height: anchoredPreview?.height ?? _previewBlockSize,
                duration:
                    anchoredPreview != null || anchorMoved || returningToEmoji
                    ? Duration.zero
                    : reduceMotion
                    ? Duration.zero
                    : const Duration(milliseconds: 150),
                child: Center(
                  child: AnimatedBuilder(
                    animation: curvedEntrance,
                    child: Opacity(
                      opacity: previewOpacity,
                      child: KeyedSubtree(
                        key: mode == ProfileAvatarMode.animated
                            ? null
                            : previewKey,
                        child: anchoredPreview == null
                            ? fixedPreview ?? const SizedBox.shrink()
                            : SizedBox.square(
                                dimension: anchoredPreview.width,
                                child: AnimatedBuilder(
                                  animation: transition,
                                  builder: (context, _) => FittedBox(
                                    child: SizedBox.square(
                                      dimension: _previewSize,
                                      child: Stack(
                                        fit: StackFit.expand,
                                        children: [
                                          if (returningToEmpty)
                                            Opacity(
                                              key: const ValueKey(
                                                'avatar-return-empty-opacity',
                                              ),
                                              opacity: 1 - transition.value,
                                              child: emptyPreview,
                                            ),
                                          if (fixedPreview != null)
                                            Opacity(
                                              key: const ValueKey(
                                                'avatar-return-draft-opacity',
                                              ),
                                              opacity: returningToEmpty
                                                  ? transition.value
                                                  : 1,
                                              child: fixedPreview,
                                            ),
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                      ),
                    ),
                    builder: (context, child) {
                      final progress = reduceMotion
                          ? 1.0
                          : curvedEntrance.value;
                      final entranceOffset = anchoredPreview == null
                          ? collapsedPreviewOffset
                          : anchoredPreview.top - displayedPreviewTop;
                      return Transform.translate(
                        key: const ValueKey('avatar-editor-entrance-transform'),
                        offset: Offset(0, entranceOffset * (1 - progress)),
                        child: Transform.scale(
                          scale: anchoredPreview != null
                              ? initialPreviewScale +
                                    (1 - initialPreviewScale) * progress
                              : 128 / _previewSize +
                                    (1 - 128 / _previewSize) * progress,
                          child: child,
                        ),
                      );
                    },
                  ),
                ),
              ),
            AnimatedPositioned(
              // Camera controls already morph into the source buttons before
              // closing. Keep their bottom rail fixed during the handoff.
              duration:
                  anchoredPreview != null ||
                      mode == ProfileAvatarMode.image ||
                      reduceMotion
                  ? Duration.zero
                  : const Duration(milliseconds: 150),
              curve: Curves.easeOutCubic,
              left: Grid.gutter,
              right: Grid.gutter,
              top: modeTop,
              height: modeHeight,
              child: FadeTransition(
                opacity: curvedEntrance,
                child:
                    anchoredPreview == null ||
                        mode == ProfileAvatarMode.animated
                    ? transitionedModeContent
                    : ClipRect(
                        child: SingleChildScrollView(
                          child: SizedBox(
                            height: contentHeight,
                            child: transitionedModeContent,
                          ),
                        ),
                      ),
              ),
            ),
            if (mode == ProfileAvatarMode.animated &&
                !reduceMotion &&
                modeTransitionProgress < 1 &&
                retainedPreview.value != null)
              Positioned(
                key: const ValueKey('avatar-mode-retained-preview'),
                left: Grid.gutter,
                right: Grid.gutter,
                top: anchoredPreview != null
                    ? displayedPreviewTop
                    : retainedPreviewTop.value +
                          ((anchoredPreview == null
                                      ? basePreviewTop
                                      : previewTop) -
                                  retainedPreviewTop.value) *
                              modeTransitionProgress,
                height: anchoredPreview?.height ?? _previewBlockSize,
                child: IgnorePointer(
                  child: Opacity(
                    opacity: 1 - modeTransitionProgress,
                    child: Center(child: retainedPreview.value),
                  ),
                ),
              ),
            if (error.value != null)
              Positioned(
                left: Grid.gutter,
                right: Grid.gutter,
                bottom: controlsBottom + _editorRailHeight,
                child: Semantics(
                  liveRegion: true,
                  child: Text(
                    error.value!,
                    textAlign: TextAlign.center,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.error,
                    ),
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
                    ),
                  ),
                ),
              ),
              Row(
                children: [
                  for (final mode in ProfileAvatarMode.values)
                    Expanded(
                      child: Semantics(
                        label: switch (mode) {
                          ProfileAvatarMode.image => 'Image',
                          ProfileAvatarMode.emoji => 'Emoji',
                          ProfileAvatarMode.animated => 'Animated',
                        },
                        button: true,
                        selected: mode == selected,
                        onTap: () => onSelected(mode),
                        child: ExcludeSemantics(
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
