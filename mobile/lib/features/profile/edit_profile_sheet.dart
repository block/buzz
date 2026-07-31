import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'profile_provider.dart';
import 'user_profile.dart';

const _profileAvatarMaxDimension = 512.0;
const _profileAvatarImageQuality = 85;

typedef PickProfileAvatarImage =
    Future<XFile?> Function({
      required double maxWidth,
      required double maxHeight,
      required int imageQuality,
    });

final profileAvatarImagePickerProvider = Provider<PickProfileAvatarImage>((
  ref,
) {
  final picker = ImagePicker();
  return ({required maxWidth, required maxHeight, required imageQuality}) =>
      picker.pickImage(
        source: ImageSource.gallery,
        maxWidth: maxWidth,
        maxHeight: maxHeight,
        imageQuality: imageQuality,
        requestFullMetadata: false,
      );
});

final profileAvatarPickerProvider = Provider<Future<String?> Function()>((ref) {
  return () async {
    final image = await ref.read(profileAvatarImagePickerProvider)(
      maxWidth: _profileAvatarMaxDimension,
      maxHeight: _profileAvatarMaxDimension,
      imageQuality: _profileAvatarImageQuality,
    );
    if (image == null) return null;
    final descriptor = await ref
        .read(mediaUploadServiceProvider)
        .uploadProfileImage(image);
    return descriptor.url;
  };
});

void showEditProfileSheet(
  BuildContext context, {
  required UserProfile? currentProfile,
}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _EditProfileSheet(currentProfile: currentProfile),
  );
}

class _EditProfileSheet extends HookConsumerWidget {
  const _EditProfileSheet({required this.currentProfile});

  final UserProfile? currentProfile;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final displayNameController = useTextEditingController(
      text: currentProfile?.displayName ?? '',
    );
    final aboutController = useTextEditingController(
      text: currentProfile?.about ?? '',
    );
    final displayName = useState(currentProfile?.displayName ?? '');
    final about = useState(currentProfile?.about ?? '');
    final avatarUrl = useState(currentProfile?.avatarUrl ?? '');
    final isUploading = useState(false);
    final isSaving = useState(false);
    final errorMessage = useState<String?>(null);

    useEffect(() {
      void displayNameListener() =>
          displayName.value = displayNameController.text;
      void aboutListener() => about.value = aboutController.text;
      displayNameController.addListener(displayNameListener);
      aboutController.addListener(aboutListener);
      return () {
        displayNameController.removeListener(displayNameListener);
        aboutController.removeListener(aboutListener);
      };
    }, [displayNameController, aboutController]);

    final nextDisplayName = displayName.value.trim();
    final hasChanges =
        nextDisplayName != (currentProfile?.displayName ?? '') ||
        avatarUrl.value.trim() != (currentProfile?.avatarUrl ?? '') ||
        about.value.trim() != (currentProfile?.about ?? '');
    final canSave =
        nextDisplayName.isNotEmpty &&
        hasChanges &&
        !isUploading.value &&
        !isSaving.value;

    Future<void> chooseAvatar() async {
      if (isUploading.value || isSaving.value) return;
      isUploading.value = true;
      errorMessage.value = null;
      try {
        final url = await ref.read(profileAvatarPickerProvider)();
        if (url != null) avatarUrl.value = url;
      } catch (_) {
        errorMessage.value = 'Couldn\u2019t upload that photo. Try again.';
      } finally {
        isUploading.value = false;
      }
    }

    Future<void> save() async {
      if (!canSave) return;
      isSaving.value = true;
      errorMessage.value = null;
      try {
        await ref
            .read(profileProvider.notifier)
            .updateProfile(
              displayName: nextDisplayName,
              avatarUrl: avatarUrl.value,
              about: about.value,
            );
        if (!context.mounted) return;
        final messenger = ScaffoldMessenger.of(context);
        Navigator.of(context).pop();
        messenger.showSnackBar(const SnackBar(content: Text('Profile saved')));
      } catch (_) {
        errorMessage.value = 'Couldn\u2019t save your profile. Try again.';
      } finally {
        isSaving.value = false;
      }
    }

    final avatarFallback = nextDisplayName.isNotEmpty
        ? nextDisplayName.characters.first.toUpperCase()
        : '?';

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        Grid.gutter +
            math.max(
              MediaQuery.viewInsetsOf(context).bottom,
              MediaQuery.viewPaddingOf(context).bottom,
            ),
      ),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Edit profile', style: context.textTheme.titleMedium),
            const SizedBox(height: Grid.twelve),
            Center(
              child: Column(
                children: [
                  ClipOval(
                    child: SizedBox.square(
                      dimension: 96,
                      child: ColoredBox(
                        color: context.colors.primaryContainer,
                        child: AvatarImageContent(
                          imageUrl: avatarUrl.value,
                          fallback: Center(
                            child: Text(
                              avatarFallback,
                              style: context.textTheme.displaySmall?.copyWith(
                                color: context.colors.onPrimaryContainer,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(height: Grid.half),
                  Wrap(
                    alignment: WrapAlignment.center,
                    spacing: Grid.xxs,
                    children: [
                      TextButton.icon(
                        onPressed: isUploading.value ? null : chooseAvatar,
                        icon: isUploading.value
                            ? const SizedBox.square(
                                dimension: 16,
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                ),
                              )
                            : const Icon(LucideIcons.imagePlus, size: 18),
                        label: Text(
                          isUploading.value
                              ? 'Uploading\u2026'
                              : 'Choose photo',
                        ),
                      ),
                      if (avatarUrl.value.isNotEmpty)
                        TextButton(
                          onPressed: isUploading.value || isSaving.value
                              ? null
                              : () => avatarUrl.value = '',
                          child: const Text('Remove'),
                        ),
                    ],
                  ),
                ],
              ),
            ),
            const SizedBox(height: Grid.xs),
            TextField(
              controller: displayNameController,
              autofocus: true,
              decoration: const InputDecoration(
                labelText: 'Display name',
                hintText: 'How others see you',
              ),
              maxLength: 80,
              textInputAction: TextInputAction.next,
            ),
            const SizedBox(height: Grid.xxs),
            TextField(
              controller: aboutController,
              decoration: const InputDecoration(
                labelText: 'Bio',
                hintText: 'A little about you',
                alignLabelWithHint: true,
              ),
              minLines: 3,
              maxLines: 5,
              maxLength: 500,
              textCapitalization: TextCapitalization.sentences,
            ),
            if (errorMessage.value case final message?)
              Padding(
                padding: const EdgeInsets.only(bottom: Grid.xs),
                child: Text(
                  message,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
            SizedBox(
              width: double.infinity,
              height: 52,
              child: FilledButton(
                onPressed: canSave ? save : null,
                child: isSaving.value
                    ? const SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Text('Save'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
