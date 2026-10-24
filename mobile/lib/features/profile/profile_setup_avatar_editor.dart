import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';

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
  });
  final ProfileAvatarDraft? initialDraft;
  final ValueChanged<ProfileAvatarDraft?>? onFinished;
  final Future<void> Function(Rect)? onPreviewReady;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final previewVisible = useState(onPreviewReady == null);
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
    final draft = useState(initialDraft);
    final mode = useState(switch (initialDraft) {
      ProfileUrlAvatarDraft() => ProfileAvatarMode.emoji,
      ProfileAnimatedAvatarDraft() => ProfileAvatarMode.animated,
      _ => ProfileAvatarMode.image,
    });
    final prepare = useState<Future<ProfileAvatarDraft?> Function()?>(null);
    final busy = useState(false);
    final error = useState(false);
    final transition = useAnimationController(initialValue: 1);
    final VoidCallback? close = busy.value
        ? null
        : () {
            if (onFinished != null) {
              onFinished!(null);
            } else {
              Navigator.of(context).pop();
            }
          };
    final VoidCallback? done =
        busy.value || (draft.value == null && prepare.value == null)
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
