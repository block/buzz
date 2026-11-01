part of '../message_actions.dart';

class _ReadAloudTile extends ConsumerWidget {
  final TimelineMessage message;

  /// Survives the sheet pop, used for the failure snackbar.
  final BuildContext pageContext;

  const _ReadAloudTile({required this.message, required this.pageContext});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final speaking = ref.watch(readAloudControllerProvider) == message.id;
    return ListTile(
      leading: Icon(speaking ? BuzzIcons.volumeX : BuzzIcons.volume2),
      title: Text(speaking ? 'Stop reading' : 'Read aloud'),
      onTap: () {
        if (!_selectMessageAction(context)) return;
        final notifier = ref.read(readAloudControllerProvider.notifier);
        if (speaking) {
          unawaited(notifier.stop());
          return;
        }
        unawaited(_speak(notifier));
      },
    );
  }

  Future<void> _speak(ReadAloudController notifier) async {
    final outcome = await notifier.toggle(
      messageId: message.id,
      content: message.content,
      tags: message.tags,
    );
    if (!pageContext.mounted) return;
    final notice = switch (outcome) {
      ReadAloudOutcome.nothingToRead => 'Nothing to read aloud',
      ReadAloudOutcome.failed => "Couldn't read this message",
      _ => null,
    };
    if (notice == null) return;
    ScaffoldMessenger.of(pageContext).showSnackBar(
      SnackBar(content: Text(notice), duration: const Duration(seconds: 2)),
    );
  }
}
