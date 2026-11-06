part of '../settings_page.dart';

class _VoiceSection extends HookConsumerWidget {
  const _VoiceSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final enabled = ref.watch(holdToTalkProvider);
    final saving = useState(false);
    Future<void> setEnabled(bool value) async {
      if (saving.value) return;
      saving.value = true;
      try {
        await ref.read(holdToTalkProvider.notifier).setEnabled(value);
      } catch (_) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            const SnackBar(
              content: Text('Could not save the voice setting. Try again.'),
            ),
            kind: ToastKind.error,
          );
        }
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    return AppListCard(
      label: 'Voice notes',
      verticalPadding: Grid.twelve,
      children: [
        AppToggleRow(
          key: const ValueKey('hold-to-talk-setting'),
          title: 'Hold to talk to Bestie',
          subtitle: 'Hold Bestie to record. Release to send.',
          value: enabled,
          onChanged: saving.value ? null : setEnabled,
        ),
      ],
    );
  }
}
