import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/profile/user_profile.dart';
import '../../shared/relay/relay.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/playing_avatar_image.dart';
import 'ios_profile_text_editor.dart';
import 'image_avatar_capture.dart';
import 'profile_avatar_editor.dart';
import 'profile_avatar_draft.dart';
import 'profile_provider.dart';
import 'profile_text_editor.dart';

/// Edits the current user's public profile metadata.
class ProfileEditPage extends HookConsumerWidget {
  /// Creates the profile details and avatar editing page.
  const ProfileEditPage({
    super.key,
    this.startInPhotoEditor = false,
    this.animatedAvatarCaptureBuilder,
    this.imageAvatarCaptureBuilder,
  });

  /// Opens directly into the photo editor when launched from Settings.
  final bool startInPhotoEditor;

  /// Overrides animated capture for focused integration tests.
  final AnimatedAvatarCaptureBuilder? animatedAvatarCaptureBuilder;

  /// Overrides still-image capture for focused integration tests.
  final ImageAvatarCaptureBuilder? imageAvatarCaptureBuilder;

  static const _avatarRadius = 64.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final profileAsync = ref.watch(profileProvider);
    final profile = profileAsync.asData?.value;
    final profileHydrated = profileAsync.hasValue;
    final isEditingAvatar = useState(startInPhotoEditor);
    final avatarDraft = useState<ProfileAvatarDraft?>(null);
    final avatarDraftMode = useState<ProfileAvatarMode?>(null);
    final avatarEditConfig = useRef<RelayConfig?>(
      startInPhotoEditor ? ref.read(relayConfigProvider) : null,
    );
    final isSavingAvatar = useState(false);
    final isClosingAvatar = useState(false);
    final prepareAnimatedAvatar =
        useRef<Future<ProfileAvatarDraft?> Function()?>(null);
    final avatarSaveError = useState<String?>(null);
    final canPrepareAnimatedAvatar = useState(false);
    final isImageCameraActive = useState(false);
    final avatarMode = useState(ProfileAvatarMode.image);
    final avatarTransition = useAnimationController(
      duration: reduceMotion
          ? Duration.zero
          : const Duration(milliseconds: 200),
    );
    useEffect(() {
      if (!startInPhotoEditor) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) unawaited(avatarTransition.forward(from: 0));
      });
      return null;
    }, const []);
    // Warm the emoji index while the profile is visible so the first editor
    // transition never waits for asset decoding.
    ref.watch(emojiDatasetProvider);

    Future<void> showFlutterFieldEditor({
      required String title,
      required String initialValue,
      required String hintText,
      required Future<void> Function(String value) onSave,
      bool multiline = false,
    }) => showBuzzModalBottomSheet<void>(
      context: context,
      title: title,
      isScrollControlled: true,
      requestFocus: true,
      builder: (_) => _ProfileTextEditSheet(
        initialValue: initialValue,
        hintText: hintText,
        multiline: multiline,
        onSave: onSave,
      ),
    );

    Future<void> editField({
      required String title,
      required String initialValue,
      required String hintText,
      required Future<void> Function(String value) onSave,
      bool multiline = false,
    }) async {
      if (defaultTargetPlatform == TargetPlatform.iOS) {
        final sheetTheme = utilitySurfaceThemeData(Theme.of(context));
        try {
          await IosProfileTextEditor.presentUntilSaved(
            title: title,
            initialValue: initialValue,
            placeholder: hintText,
            multiline: multiline,
            brightness: sheetTheme.brightness,
            pageBackgroundColor: sheetTheme.colorScheme.surface,
            containerBackgroundColor:
                sheetTheme.colorScheme.surfaceContainerHighest,
            onSave: onSave,
            shouldRetryOnError: (error) =>
                error is! ProfileCommunityChangedException,
            canPresent: () =>
                context.mounted && (ModalRoute.of(context)?.isCurrent ?? true),
            onSaveError: () {
              if (!context.mounted ||
                  !(ModalRoute.of(context)?.isCurrent ?? true)) {
                return;
              }
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(
                  content: Text("We couldn't save this change. Try again."),
                ),
              );
            },
          );
          return;
        } on MissingPluginException {
          // Keep the Flutter editor available in previews and older builds.
        } on PlatformException {
          // Fall back when the native presenter is temporarily unavailable.
        }
      }

      await showFlutterFieldEditor(
        title: title,
        initialValue: initialValue,
        hintText: hintText,
        multiline: multiline,
        onSave: onSave,
      );
    }

    void openAvatarEditor() {
      if (!profileHydrated) return;
      avatarEditConfig.value = ref.read(relayConfigProvider);
      isEditingAvatar.value = true;
      unawaited(avatarTransition.forward(from: 0));
    }

    Future<void> closeAvatarEditor({bool whileSaving = false}) async {
      if ((isSavingAvatar.value && !whileSaving) || isClosingAvatar.value) {
        return;
      }
      isClosingAvatar.value = true;
      await avatarTransition.reverse();
      if (!context.mounted) return;
      if (startInPhotoEditor) {
        Navigator.of(context).pop();
        return;
      }
      isEditingAvatar.value = false;
      avatarDraft.value = null;
      avatarDraftMode.value = null;
      avatarEditConfig.value = null;
      prepareAnimatedAvatar.value = null;
      canPrepareAnimatedAvatar.value = false;
      isImageCameraActive.value = false;
      avatarMode.value = ProfileAvatarMode.image;
      isClosingAvatar.value = false;
    }

    Future<void> saveAvatar() async {
      if (isSavingAvatar.value || isClosingAvatar.value) return;
      final openingConfig = avatarEditConfig.value;
      if (openingConfig == null) return;
      final saveConfig = ref.read(relayConfigProvider);
      final uploadService = ref.read(mediaUploadServiceProvider);

      void requireCurrentCommunity() {
        final currentConfig = ref.read(relayConfigProvider);
        if (currentConfig.storedOrigin != openingConfig.storedOrigin ||
            currentConfig.nsec != openingConfig.nsec ||
            currentConfig.storedOrigin != saveConfig.storedOrigin ||
            currentConfig.nsec != saveConfig.nsec ||
            !identical(ref.read(mediaUploadServiceProvider), uploadService)) {
          throw ProfileCommunityChangedException();
        }
      }

      Future<void> discardStaleEditor() async {
        avatarDraft.value = null;
        avatarDraftMode.value = null;
        prepareAnimatedAvatar.value = null;
        canPrepareAnimatedAvatar.value = false;
        if (context.mounted) await closeAvatarEditor(whileSaving: true);
      }

      isSavingAvatar.value = true;
      avatarSaveError.value = null;
      try {
        requireCurrentCommunity();
        var nextDraft = avatarDraftMode.value == avatarMode.value
            ? avatarDraft.value
            : null;
        if (avatarMode.value == ProfileAvatarMode.animated &&
            nextDraft == null) {
          nextDraft = await prepareAnimatedAvatar.value?.call();
          requireCurrentCommunity();
          if (nextDraft != null) {
            avatarDraft.value = nextDraft;
            avatarDraftMode.value = ProfileAvatarMode.animated;
          }
        }
        if (nextDraft == null) return;
        requireCurrentCommunity();
        final nextAvatar = await nextDraft.upload(uploadService);
        requireCurrentCommunity();
        await ref.read(profileProvider.notifier).updateAvatarUrl(nextAvatar);
        requireCurrentCommunity();
        if (nextDraft is ProfileAnimatedAvatarDraft) {
          ref
              .read(profileAvatarHandoffProvider.notifier)
              .show(
                ProfileAvatarHandoff(
                  avatarUrl: nextAvatar,
                  animation: nextDraft.animation,
                  poster: nextDraft.poster,
                ),
              );
        } else {
          ref.read(profileAvatarHandoffProvider.notifier).clearAny();
        }
        if (context.mounted) await closeAvatarEditor(whileSaving: true);
      } on ProfileCommunityChangedException {
        await discardStaleEditor();
      } catch (_) {
        try {
          requireCurrentCommunity();
        } on ProfileCommunityChangedException {
          await discardStaleEditor();
          return;
        }
        avatarSaveError.value =
            "We couldn't save your profile photo. Try again.";
      } finally {
        if (context.mounted) isSavingAvatar.value = false;
      }
    }

    final canSaveAvatar =
        profileHydrated &&
        !isImageCameraActive.value &&
        (avatarMode.value == ProfileAvatarMode.animated
            ? canPrepareAnimatedAvatar.value
            : avatarDraftMode.value == avatarMode.value &&
                  avatarDraft.value != null);
    final activeDraft = avatarDraftMode.value == avatarMode.value
        ? avatarDraft.value
        : null;
    final avatarHandoff = ref.watch(profileAvatarHandoffProvider);

    return PopScope(
      canPop: !isEditingAvatar.value,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && isEditingAvatar.value) {
          unawaited(closeAvatarEditor());
        }
      },
      child: FrostedScaffold(
        useUtilitySurfaceTheme: true,
        resizeToAvoidBottomInset: isEditingAvatar.value ? false : null,
        appBar: FrostedAppBar(
          centerTitle: true,
          title: AnimatedSwitcher(
            duration: reduceMotion
                ? const Duration(milliseconds: 120)
                : const Duration(milliseconds: 220),
            child: Text(
              isEditingAvatar.value ? 'Edit Photo' : 'Profile',
              key: ValueKey(isEditingAvatar.value),
            ),
          ),
          leading: isEditingAvatar.value
              ? defaultTargetPlatform == TargetPlatform.iOS
                    ? IosGlassNavigationButton(
                        key: const ValueKey('avatar-editor-back'),
                        icon: IosGlassNavigationIcon.back,
                        semanticLabel: 'Back to profile',
                        onPressed: isClosingAvatar.value
                            ? null
                            : () => unawaited(closeAvatarEditor()),
                      )
                    : IconButton(
                        key: const ValueKey('avatar-editor-back'),
                        tooltip: 'Back to profile',
                        onPressed: isClosingAvatar.value
                            ? null
                            : () => unawaited(closeAvatarEditor()),
                        icon: const Icon(LucideIcons.arrowLeft),
                      )
              : null,
          actions: isEditingAvatar.value
              ? [
                  if (defaultTargetPlatform == TargetPlatform.iOS)
                    IosGlassNavigationAction(
                      key: const ValueKey('avatar-save'),
                      label: 'Save',
                      width: 72,
                      isBusy: isSavingAvatar.value,
                      onPressed:
                          canSaveAvatar &&
                              !isSavingAvatar.value &&
                              !isClosingAvatar.value
                          ? () {
                              unawaited(HapticFeedback.lightImpact());
                              unawaited(saveAvatar());
                            }
                          : null,
                    )
                  else
                    Padding(
                      padding: const EdgeInsets.only(right: Grid.xs),
                      child: _ProfileActionPill(
                        key: const ValueKey('avatar-save'),
                        label: 'Save',
                        isBusy: isSavingAvatar.value,
                        onTap:
                            canSaveAvatar &&
                                !isSavingAvatar.value &&
                                !isClosingAvatar.value
                            ? () {
                                unawaited(HapticFeedback.lightImpact());
                                unawaited(saveAvatar());
                              }
                            : null,
                      ),
                    ),
                ]
              : const [],
        ),
        body: isEditingAvatar.value
            ? Stack(
                children: [
                  IgnorePointer(
                    ignoring: !profileHydrated || isSavingAvatar.value,
                    child: LayoutBuilder(
                      builder: (context, constraints) => SingleChildScrollView(
                        key: const ValueKey('avatar-editor-scroll-view'),
                        physics: constraints.maxHeight < 600
                            ? const ClampingScrollPhysics()
                            : const NeverScrollableScrollPhysics(),
                        child: SizedBox(
                          height: constraints.maxHeight < 600
                              ? 700
                              : constraints.maxHeight,
                          child: ProfileAvatarEditor(
                            key: const ValueKey('profile-avatar-editor-page'),
                            currentAvatarUrl: profile?.avatarUrl,
                            fallbackInitial: profile?.initial ?? '?',
                            draft: activeDraft,
                            mode: avatarMode.value,
                            transition: avatarTransition,
                            onModeChanged: (nextMode) =>
                                avatarMode.value = nextMode,
                            onDraftChanged: (draft) {
                              avatarDraft.value = draft;
                              avatarDraftMode.value = draft == null
                                  ? null
                                  : avatarMode.value;
                            },
                            onAnimatedPrepareChanged: (prepare) {
                              prepareAnimatedAvatar.value = prepare;
                              canPrepareAnimatedAvatar.value = prepare != null;
                              if (avatarMode.value ==
                                  ProfileAvatarMode.animated) {
                                avatarDraft.value = null;
                                avatarDraftMode.value = null;
                              }
                            },
                            onImageCameraActiveChanged: (active) {
                              isImageCameraActive.value = active;
                            },
                            animatedCaptureBuilder:
                                animatedAvatarCaptureBuilder,
                            imageCaptureBuilder: imageAvatarCaptureBuilder,
                          ),
                        ),
                      ),
                    ),
                  ),
                  if (!profileHydrated || avatarSaveError.value != null)
                    Positioned(
                      left: Grid.gutter,
                      right: Grid.gutter,
                      bottom: Grid.xl,
                      child: Semantics(
                        liveRegion: true,
                        child: Text(
                          avatarSaveError.value ??
                              (profileAsync.hasError
                                  ? "We couldn't load your profile. Go back and try again."
                                  : 'Loading your profile…'),
                          textAlign: TextAlign.center,
                          style: context.textTheme.bodyMedium?.copyWith(
                            color:
                                avatarSaveError.value != null ||
                                    profileAsync.hasError
                                ? context.colors.error
                                : context.colors.onSurfaceVariant,
                          ),
                        ),
                      ),
                    ),
                ],
              )
            : ListView(
                key: const ValueKey('profile-information-page'),
                padding: EdgeInsets.only(
                  top: frostedAppBarHeight(context) + Grid.sm,
                  bottom: Grid.sm,
                ),
                children: [
                  _ProfilePhotoEditor(
                    profile: profile,
                    handoff: avatarHandoff,
                    onEditPhoto: profileHydrated ? openAvatarEditor : null,
                  ),
                  AppListCard(
                    key: const ValueKey('profile-info-card'),
                    dividerIndent: Grid.xs,
                    verticalPadding: Grid.sm,
                    children: [
                      AppListRow(
                        key: const ValueKey('profile-display-name-row'),
                        title: 'Display name',
                        subtitle: _fieldValue(profile?.displayName),
                        subtitleMaxLines: 1,
                        trailing: const _EditChevron(),
                        onTap: !profileHydrated
                            ? null
                            : () {
                                final container = ProviderScope.containerOf(
                                  context,
                                  listen: false,
                                );
                                unawaited(
                                  editField(
                                    title: 'Display name',
                                    initialValue: profile?.displayName ?? '',
                                    hintText: 'Display name',
                                    onSave: bindProfileSaveToOpeningContext(
                                      container,
                                      container
                                          .read(profileProvider.notifier)
                                          .updateDisplayName,
                                    ),
                                  ),
                                );
                              },
                      ),
                      AppListRow(
                        key: const ValueKey('profile-description-row'),
                        title: 'Profile description',
                        subtitle: _fieldValue(profile?.about),
                        subtitleMaxLines: 3,
                        trailing: const _EditChevron(),
                        onTap: !profileHydrated
                            ? null
                            : () {
                                final container = ProviderScope.containerOf(
                                  context,
                                  listen: false,
                                );
                                unawaited(
                                  editField(
                                    title: 'Profile description',
                                    initialValue: profile?.about ?? '',
                                    hintText: 'Profile description',
                                    multiline: true,
                                    onSave: bindProfileSaveToOpeningContext(
                                      container,
                                      container
                                          .read(profileProvider.notifier)
                                          .updateAbout,
                                    ),
                                  ),
                                );
                              },
                      ),
                    ],
                  ),
                ],
              ),
      ),
    );
  }
}

String _fieldValue(String? value) {
  final trimmed = value?.trim() ?? '';
  return trimmed.isEmpty ? 'Not set' : trimmed;
}

class _ProfilePhotoEditor extends ConsumerWidget {
  const _ProfilePhotoEditor({
    required this.profile,
    required this.handoff,
    required this.onEditPhoto,
  });

  final UserProfile? profile;
  final ProfileAvatarHandoff? handoff;
  final VoidCallback? onEditPhoto;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activeHandoff = handoff?.avatarUrl == profile?.avatarUrl
        ? handoff
        : null;
    return Column(
      children: [
        PlayingAvatarImage(
          key: const ValueKey('profile-edit-avatar'),
          imageUrl: profile?.avatarUrl,
          radius: ProfileEditPage._avatarRadius,
          backgroundColor: context.colors.primaryContainer,
          loadingImage: activeHandoff == null
              ? null
              : MemoryImage(activeHandoff.animation),
          loadingPosterImage: activeHandoff == null
              ? null
              : MemoryImage(activeHandoff.poster),
          onAnimationReady: activeHandoff == null
              ? null
              : () => ref
                    .read(profileAvatarHandoffProvider.notifier)
                    .clear(activeHandoff.avatarUrl),
          fallback: Text(
            profile?.initial ?? '?',
            style: context.textTheme.displaySmall?.copyWith(
              color: context.colors.onPrimaryContainer,
            ),
          ),
        ),
        const SizedBox(height: Grid.twelve),
        _ProfileActionPill(
          key: const ValueKey('profile-edit-photo-pill'),
          semanticLabel: 'Edit profile photo',
          label: 'Edit Photo',
          onTap: onEditPhoto,
        ),
      ],
    );
  }
}

class _ProfileActionPill extends StatelessWidget {
  const _ProfileActionPill({
    super.key,
    required this.label,
    required this.onTap,
    this.semanticLabel,
    this.isBusy = false,
  });

  final String label;
  final String? semanticLabel;
  final VoidCallback? onTap;
  final bool isBusy;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    enabled: onTap != null,
    label: semanticLabel ?? label,
    child: Material(
      color: onTap == null
          ? context.colors.surfaceContainerHighest.withValues(alpha: 0.5)
          : context.colors.surfaceContainerHighest,
      borderRadius: BorderRadius.circular(Radii.full),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xs,
            vertical: Grid.xxs,
          ),
          child: isBusy
              ? const BuzzLoadingIndicator(
                  size: 18,
                  semanticLabel: 'Saving profile photo',
                )
              : Text(
                  label,
                  style: context.textTheme.labelLarge?.copyWith(
                    color: onTap == null
                        ? context.colors.onSurfaceVariant
                        : context.colors.onSurface,
                    fontWeight: FontWeight.w600,
                  ),
                ),
        ),
      ),
    ),
  );
}

class _EditChevron extends StatelessWidget {
  const _EditChevron();

  @override
  Widget build(BuildContext context) => Icon(
    LucideIcons.chevronRight,
    size: 18,
    color: context.colors.onSurfaceVariant,
  );
}

class _ProfileTextEditSheet extends HookWidget {
  const _ProfileTextEditSheet({
    required this.initialValue,
    required this.hintText,
    required this.multiline,
    required this.onSave,
  });

  final String initialValue;
  final String hintText;
  final bool multiline;
  final Future<void> Function(String value) onSave;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);
    useListenable(controller);
    final isSaving = useState(false);
    final error = useState<String?>(null);
    final hasChanges = controller.text.trim() != initialValue.trim();

    Future<void> save() async {
      if (!hasChanges || isSaving.value) return;
      isSaving.value = true;
      error.value = null;
      try {
        await onSave(controller.text);
        if (context.mounted) Navigator.of(context).pop();
      } on ProfileCommunityChangedException {
        if (context.mounted) Navigator.of(context).pop();
      } catch (_) {
        error.value = "We couldn't save this change. Try again.";
      } finally {
        if (context.mounted) isSaving.value = false;
      }
    }

    return SafeArea(
      top: false,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xxs,
          Grid.gutter,
          MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              key: const ValueKey('profile-field-input'),
              controller: controller,
              autofocus: true,
              enabled: !isSaving.value,
              minLines: multiline ? 4 : 1,
              maxLines: multiline ? 6 : 1,
              textCapitalization: TextCapitalization.sentences,
              textInputAction: multiline
                  ? TextInputAction.newline
                  : TextInputAction.done,
              onSubmitted: multiline ? null : (_) => unawaited(save()),
              decoration: InputDecoration(hintText: hintText),
            ),
            if (error.value != null) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                error.value!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.xs),
            FilledButton(
              key: const ValueKey('profile-field-save'),
              onPressed: hasChanges && !isSaving.value ? save : null,
              child: Text(isSaving.value ? 'Saving…' : 'Save'),
            ),
          ],
        ),
      ),
    );
  }
}
