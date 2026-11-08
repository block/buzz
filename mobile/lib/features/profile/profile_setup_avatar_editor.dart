import 'dart:math';

import 'package:flutter/material.dart';

import '../../shared/emoji/emoji_avatar.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import 'profile_setup_empty_avatar.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'profile_avatar_draft.dart';
import 'profile_avatar_editor.dart';

/// Reuses the profile editor while returning a local draft instead of publishing.
class ProfileSetupAvatarEditor extends HookConsumerWidget {
  const ProfileSetupAvatarEditor({
    super.key,
    this.initialDraft,
    this.onFinished,
    this.onPreviewReady,
    this.anchoredPreview,
  });
  final ProfileAvatarDraft? initialDraft;
  final Rect? anchoredPreview;
  final ValueChanged<ProfileAvatarDraft?>? onFinished;
  final Future<void> Function(Rect)? onPreviewReady;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final previewBounds = useState(anchoredPreview);
    final previewVisible = useState(
      onPreviewReady == null || anchoredPreview != null,
    );
    final previewKey = useMemoized(() => GlobalKey());
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) async {
        final box = previewKey.currentContext?.findRenderObject();
        if (box is RenderBox && box.hasSize) {
          try {
            await onPreviewReady
                ?.call(box.localToGlobal(Offset.zero) & box.size)
                .timeout(const Duration(seconds: 2), onTimeout: () {});
          } finally {
            if (context.mounted) previewVisible.value = true;
          }
        }
      });
      return null;
    }, const []);
    final openingDraft = useMemoized(
      () =>
          initialDraft ??
          ProfileUrlAvatarDraft(
            emojiAvatarDataUrl('😊', emojiAvatarColors[Random().nextInt(18)]),
          ),
      const [],
    );
    final draft = useState<ProfileAvatarDraft?>(openingDraft);
    final mode = useState(switch (openingDraft) {
      ProfileUrlAvatarDraft() => ProfileAvatarMode.emoji,
      ProfileAnimatedAvatarDraft() => ProfileAvatarMode.animated,
      _ => ProfileAvatarMode.image,
    });
    final prepare = useState<Future<ProfileAvatarDraft?> Function()?>(null);
    final busy = useState(false);
    final returningToEmpty = useState(false);
    final error = useState(false);
    final transition = useAnimationController(
      initialValue: anchoredPreview == null ? 1 : 0,
      duration: const Duration(milliseconds: 200),
    );
    final VoidCallback? close = busy.value
        ? null
        : () {
            if (onFinished != null) {
              returningToEmpty.value = initialDraft == null;
              onFinished!(null);
            } else {
              Navigator.of(context).pop();
            }
          };
    final routeIsCurrent = ModalRoute.isCurrentOf(context) ?? true;
    final VoidCallback? done =
        busy.value ||
            !routeIsCurrent ||
            (draft.value == null && prepare.value == null)
        ? null
        : () async {
            busy.value = true;
            error.value = false;
            try {
              final result =
                  mode.value == ProfileAvatarMode.animated &&
                      prepare.value != null
                  ? await prepare.value!()
                  : draft.value;
              if (context.mounted && result != null) {
                if (onFinished != null) {
                  onFinished!(result);
                } else {
                  Navigator.of(context).pop(result);
                }
              }
            } catch (_) {
              if (context.mounted) error.value = true;
            } finally {
              if (context.mounted) busy.value = false;
            }
          };
    final doneRef = useRef(done)..value = done;
    final closeRef = useRef(close)..value = close;
    useEffect(() {
      if (anchoredPreview == null) return null;
      const channel = MethodChannel('buzz/profile_avatar_sheet');
      channel.setMethodCallHandler((call) async {
        final reduceMotion = MediaQuery.disableAnimationsOf(context);
        if (call.method == 'done') doneRef.value?.call();
        if (call.method == 'back') {
          final navigator = Navigator.of(context);
          if (navigator.canPop()) {
            navigator.pop();
          } else {
            closeRef.value?.call();
          }
        }
        if (call.method == 'hide' || call.method == 'reveal') {
          final bounds = call.arguments;
          if (bounds is Map) {
            previewBounds.value = Rect.fromLTWH(
              (bounds['x'] as num).toDouble(),
              (bounds['y'] as num).toDouble(),
              (bounds['width'] as num).toDouble(),
              (bounds['height'] as num).toDouble(),
            );
          }
        }
        if (call.method == 'hide') {
          if (reduceMotion) {
            transition.value = 0;
          } else {
            await transition.reverse().orCancel;
          }
        }
        if (call.method == 'hide') {
          // Let UIKit replace this view only after the final avatar is painted.
          await WidgetsBinding.instance.endOfFrame;
        }
        if (call.method == 'reveal') {
          if (reduceMotion) {
            transition.value = 1;
          } else {
            transition.forward(from: 0);
          }
        }
      });
      return () => channel.setMethodCallHandler(null);
    }, const []);
    useEffect(() {
      if (anchoredPreview == null) return null;
      const channel = MethodChannel('buzz/profile_avatar_sheet');
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) {
          channel.invokeMethod<void>('controls', {
            'canDone': done != null,
            'busy': busy.value,
            'fullScreen': !routeIsCurrent,
          });
        }
      });
      return null;
    }, [done != null, busy.value, routeIsCurrent]);
    if (anchoredPreview != null) {
      return Theme(
        data: utilitySurfaceThemeData(Theme.of(context)),
        child: Scaffold(
          // Native profile chrome returns underneath the exiting controls.
          backgroundColor: Colors.transparent,
          resizeToAvoidBottomInset: false,
          body: Column(
            children: [
              if (error.value)
                const Text('Could not prepare this photo. Try again.'),
              Expanded(
                child: IgnorePointer(
                  ignoring: busy.value,
                  child: ProfileAvatarEditor(
                    anchoredPreview: previewBounds.value,
                    previewKey: previewKey,
                    previewOpacity: previewVisible.value ? 1 : 0,
                    currentAvatarUrl: initialDraft is ProfileUrlAvatarDraft
                        ? (initialDraft as ProfileUrlAvatarDraft).url
                        : null,
                    fallbackInitial: '+',
                    emptyPreview: const ProfileSetupEmptyAvatar(),
                    returningToEmpty: returningToEmpty.value,
                    draft: draft.value,
                    mode: mode.value,
                    transition: transition,
                    onModeChanged: (value) {
                      mode.value = value;
                      draft.value = null;
                      prepare.value = null;
                    },
                    onDraftChanged: (value) => draft.value = value,
                    onAnimatedPrepareChanged: (value) => prepare.value = value,
                    onImageCameraActiveChanged: (_) {},
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }
    return PopScope(
      canPop: !busy.value,
      child: FrostedScaffold(
        useUtilitySurfaceTheme: true,
        resizeToAvoidBottomInset: false,
        appBar: FrostedAppBar(
          nativeTitle: 'Profile photo',
          centerTitle: true,
          title: const Text('Profile photo'),
          leading: defaultTargetPlatform == TargetPlatform.iOS
              ? IosGlassNavigationButton(
                  icon: IosGlassNavigationIcon.back,
                  semanticLabel: 'Back to profile',
                  onPressed: close,
                )
              : IconButton(
                  onPressed: close,
                  tooltip: 'Back to profile',
                  icon: const Icon(Icons.arrow_back),
                ),
          actions: [
            if (defaultTargetPlatform == TargetPlatform.iOS)
              IosGlassNavigationAction(
                label: 'Done',
                width: 72,
                isBusy: busy.value,
                onPressed: done,
              )
            else
              TextButton(
                onPressed: done,
                child: Text(busy.value ? 'Preparing…' : 'Done'),
              ),
          ],
        ),
        body: Column(
          children: [
            if (error.value)
              const Text('Could not prepare this photo. Try again.'),
            Expanded(
              child: IgnorePointer(
                ignoring: busy.value,
                child: LayoutBuilder(
                  builder: (context, constraints) => SingleChildScrollView(
                    physics: constraints.maxHeight < 720
                        ? const ClampingScrollPhysics()
                        : const NeverScrollableScrollPhysics(),
                    child: SizedBox(
                      height: constraints.maxHeight < 720
                          ? 720
                          : constraints.maxHeight,
                      child: ProfileAvatarEditor(
                        previewKey: previewKey,
                        headerGap: Grid.lg,
                        previewOpacity: previewVisible.value ? 1 : 0,
                        currentAvatarUrl: initialDraft is ProfileUrlAvatarDraft
                            ? (initialDraft as ProfileUrlAvatarDraft).url
                            : null,
                        fallbackInitial: '+',
                        emptyPreview: const ProfileSetupEmptyAvatar(),
                        draft: draft.value,
                        mode: mode.value,
                        transition: transition,
                        onModeChanged: (value) {
                          mode.value = value;
                          draft.value = null;
                          prepare.value = null;
                        },
                        onDraftChanged: (value) => draft.value = value,
                        onAnimatedPrepareChanged: (value) {
                          prepare.value = value;
                        },
                        onImageCameraActiveChanged: (_) {},
                      ),
                    ),
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
