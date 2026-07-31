import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'profile_provider.dart';
import 'user_profile.dart';

const _avatarColors = [
  (hex: '#F4B942', color: Color(0xFFF4B942)),
  (hex: '#E66A6A', color: Color(0xFFE66A6A)),
  (hex: '#9B72CF', color: Color(0xFF9B72CF)),
  (hex: '#5B8DEF', color: Color(0xFF5B8DEF)),
  (hex: '#4EAD8C', color: Color(0xFF4EAD8C)),
];

/// Open the current user's small kind:0 profile editor.
void showEditProfileSheet(BuildContext context, {UserProfile? profile}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _EditProfileSheet(profile: profile),
  );
}

/// Build a self-contained, percent-encoded SVG avatar around [emoji].
String emojiAvatarDataUrl(String emoji, String color) {
  final escaped = emoji
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
  final svg =
      '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" '
      'viewBox="0 0 512 512"><rect width="512" height="512" rx="256" '
      'fill="$color"/><text x="50%" y="56%" dominant-baseline="middle" '
      'text-anchor="middle" font-size="258">$escaped</text></svg>';
  return 'data:image/svg+xml,${Uri.encodeComponent(svg)}';
}

class _EditProfileSheet extends HookConsumerWidget {
  const _EditProfileSheet({required this.profile});

  final UserProfile? profile;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final displayNameController = useTextEditingController(
      text: profile?.displayName ?? '',
    );
    final displayName = useState(profile?.displayName ?? '');
    final emoji = useState('');
    final colorIndex = useState(0);
    final isSaving = useState(false);
    final avatarUrl = emoji.value.trim().isEmpty
        ? profile?.avatarUrl
        : emojiAvatarDataUrl(
            emoji.value.trim(),
            _avatarColors[colorIndex.value].hex,
          );
    final initial = displayName.value.trim().isNotEmpty
        ? displayName.value.trim()[0].toUpperCase()
        : profile?.initial ?? '?';
    final canSave = displayName.value.trim().isNotEmpty && !isSaving.value;

    Future<void> save() async {
      if (!canSave) return;
      isSaving.value = true;
      final messenger = ScaffoldMessenger.of(context);
      try {
        final avatarEmoji = emoji.value.trim();
        await ref
            .read(profileProvider.notifier)
            .saveProfile(
              displayName: displayName.value,
              avatarUrl: avatarEmoji.isEmpty
                  ? null
                  : emojiAvatarDataUrl(
                      avatarEmoji,
                      _avatarColors[colorIndex.value].hex,
                    ),
            );
        if (!context.mounted) return;
        Navigator.of(context).pop();
        messenger.showSnackBar(const SnackBar(content: Text('Profile saved')));
      } catch (_) {
        if (!context.mounted) return;
        messenger.showSnackBar(
          const SnackBar(content: Text('Unable to save profile')),
        );
        isSaving.value = false;
      }
    }

    return SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        Grid.gutter +
            MediaQuery.viewInsetsOf(context).bottom +
            MediaQuery.viewPaddingOf(context).bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Edit profile', style: context.textTheme.titleLarge),
          const SizedBox(height: Grid.xs),
          Center(
            child: AvatarImage(
              imageUrl: avatarUrl,
              radius: 48,
              backgroundColor: context.colors.primaryContainer,
              fallback: Text(
                initial,
                style: context.textTheme.headlineMedium?.copyWith(
                  color: context.colors.onPrimaryContainer,
                ),
              ),
            ),
          ),
          const SizedBox(height: Grid.xs),
          TextField(
            autofocus: true,
            decoration: const InputDecoration(labelText: 'Display name'),
            textCapitalization: TextCapitalization.words,
            textInputAction: TextInputAction.next,
            onChanged: (value) => displayName.value = value,
            controller: displayNameController,
          ),
          const SizedBox(height: Grid.twelve),
          TextField(
            decoration: const InputDecoration(
              labelText: 'Avatar emoji',
              hintText: '🐝',
              helperText: 'Stored as an inline SVG — no image host needed.',
            ),
            textInputAction: TextInputAction.done,
            onChanged: (value) => emoji.value = value,
            onSubmitted: (_) => save(),
          ),
          const SizedBox(height: Grid.twelve),
          Text('Avatar color', style: context.textTheme.labelLarge),
          const SizedBox(height: Grid.xxs),
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              for (final (index, entry) in _avatarColors.indexed)
                Semantics(
                  button: true,
                  selected: colorIndex.value == index,
                  label: 'Avatar color ${index + 1}',
                  child: InkWell(
                    customBorder: const CircleBorder(),
                    onTap: () => colorIndex.value = index,
                    child: Padding(
                      padding: const EdgeInsets.all(Grid.half),
                      child: Container(
                        width: 36,
                        height: 36,
                        decoration: BoxDecoration(
                          color: entry.color,
                          shape: BoxShape.circle,
                          border: Border.all(
                            color: colorIndex.value == index
                                ? context.colors.onSurface
                                : Colors.transparent,
                            width: 2,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
          const SizedBox(height: Grid.gutter),
          SizedBox(
            width: double.infinity,
            height: 52,
            child: FilledButton(
              onPressed: canSave ? save : null,
              child: Text(isSaving.value ? 'Saving…' : 'Save profile'),
            ),
          ),
        ],
      ),
    );
  }
}
