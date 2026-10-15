part of 'me_page.dart';

Future<void> _createMeProject(BuildContext context, WidgetRef ref) async {
  final sync = ref.read(meSyncProvider);
  var intent = MeProjectCreation.pending(sync);
  if (intent != null) {
    final reviewed = intent;
    final resume = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Unfinished project'),
        content: Text(
          'Resume “${reviewed.name}”, or discard its setup to start another project. Any project already sent or awaiting confirmation is kept.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Later'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Discard setup'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Resume'),
          ),
        ],
      ),
    );
    if (resume == null || !context.mounted) return;
    if (!resume) {
      await reviewed.discard(sync);
      intent = null;
    }
  }
  if (!context.mounted) return;
  if (intent == null) {
    final name = await _name(context, 'New project');
    if (name == null) return;
    intent = MeProjectCreation(const Uuid().v4(), name);
  }
  while (context.mounted) {
    try {
      await intent!.complete(_currentMeSync(ref, sync));
      return;
    } catch (error) {
      if (error is MeRecoveryConflict) rethrow;
      // A failed preferences write can still update its optimistic cache.
      // Only advertise recovery after rereading the durable journal.
      var saved = false;
      try {
        await sync.prefs?.reload();
        saved = MeProjectCreation.pending(sync) != null;
      } catch (_) {
        // Storage could not be verified. Retain the entered form below.
      }
      if (saved) {
        throw StateError(
          'Project setup saved on this phone. Open New project to resume or discard it. ${error.toString().replaceFirst('Bad state: ', '')}',
        );
      }
      if (!context.mounted || sync.disposed) rethrow;
      final retry = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('Project was not saved'),
          content: const Text(
            'Couldn’t save your project on this phone. Try again to keep your entered name.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('Cancel'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('Try again'),
            ),
          ],
        ),
      );
      if (retry != true || !context.mounted) return;
      final name = await _name(
        context,
        'New project',
        initialValue: intent!.name,
      );
      if (name == null) return;
      intent = MeProjectCreation(intent.id, name);
    }
  }
}
