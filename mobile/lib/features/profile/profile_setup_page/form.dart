part of '../profile_setup_page.dart';

class _ProfileSetupForm extends HookWidget {
  const _ProfileSetupForm({
    required this.avatar,
    required this.onName,
    required this.editAvatar,
    required this.save,
    required this.notNow,
  });

  final ValueNotifier<ProfileAvatarDraft?> avatar;
  final ValueChanged<String> onName;
  final Future<void> Function() editAvatar;
  final Future<void> Function({bool skipAvatar}) save;
  final VoidCallback notNow;

  @override
  Widget build(BuildContext context) {
    final draft = useValueListenable(avatar);
    final name = useTextEditingController();
    final nameFocus = useFocusNode();
    useListenable(name);
    final choosingAvatar = useState(false);
    final busy = useState(false);
    final error = useState(false);
    final validName =
        name.text.trim().isNotEmpty &&
        name.text.trim().characters.length <= 100;

    void back() {
      if (busy.value) return;
      error.value = false;
      choosingAvatar.value = false;
      nameFocus.requestFocus();
    }

    Future<void> submit({bool skipAvatar = false}) async {
      if (busy.value || !validName) return;
      if (!choosingAvatar.value) {
        nameFocus.unfocus();
        choosingAvatar.value = true;
        return;
      }
      if (!skipAvatar && draft == null) return;
      busy.value = true;
      error.value = false;
      try {
        await save(skipAvatar: skipAvatar);
      } catch (_) {
        if (context.mounted) error.value = true;
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    final avatarLabel = draft == null
        ? 'Add profile photo'
        : 'Edit profile photo';
    return OnboardingStepPage(
      title: choosingAvatar.value
          ? 'Choose your avatar'
          : 'Choose a display name',
      subtitle: choosingAvatar.value
          ? 'Let’s choose an avatar so people know it’s you.'
          : 'Your display name is how people see you in Buzz. You can change it any time.',
      onBack: choosingAvatar.value && !busy.value ? back : null,
      content: choosingAvatar.value
          ? Padding(
              padding: const EdgeInsets.symmetric(vertical: Grid.lg),
              child: Center(
                child: SizedBox.square(
                  dimension: 220,
                  child: IconButton(
                    tooltip: avatarLabel,
                    onPressed: busy.value ? null : editAvatar,
                    padding: EdgeInsets.zero,
                    icon: ExcludeSemantics(
                      child: switch (draft) {
                        ProfileImageAvatarDraft(:final bytes) => CircleAvatar(
                          radius: 110,
                          backgroundImage: MemoryImage(bytes),
                        ),
                        ProfileAnimatedAvatarDraft(:final poster) =>
                          CircleAvatar(
                            radius: 110,
                            backgroundImage: MemoryImage(poster),
                          ),
                        ProfileUrlAvatarDraft(:final url) => PlayingAvatarImage(
                          imageUrl: url,
                          radius: 110,
                          fallback: const ProfileSetupEmptyAvatar(),
                        ),
                        null => const ProfileSetupEmptyAvatar(),
                      },
                    ),
                  ),
                ),
              ),
            )
          : TextField(
              controller: name,
              focusNode: nameFocus,
              autofocus: true,
              maxLength: 100,
              textCapitalization: TextCapitalization.words,
              textInputAction: TextInputAction.next,
              onSubmitted: (_) => submit(),
              onChanged: onName,
              decoration: const InputDecoration(labelText: 'Display name'),
            ),
      actions: [
        if (error.value)
          Semantics(
            liveRegion: true,
            child: const Text(
              'Could not save your profile. Try again or choose Not now to continue without saving.',
            ),
          ),
        FilledButton(
          onPressed:
              busy.value ||
                  !validName ||
                  (choosingAvatar.value && draft == null)
              ? null
              : submit,
          child: Text(busy.value ? 'Saving…' : 'Continue'),
        ),
        if (choosingAvatar.value)
          TextButton(
            onPressed: busy.value ? null : () => submit(skipAvatar: true),
            child: const Text('Skip'),
          ),
        if (error.value)
          TextButton(
            onPressed: busy.value ? null : notNow,
            child: const Text('Not now'),
          ),
      ],
    );
  }
}
