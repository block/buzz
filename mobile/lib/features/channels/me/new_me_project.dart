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
  try {
    await intent.complete(sync);
  } catch (error) {
    if (error is MeRecoveryConflict) rethrow;
    if (MeProjectCreation.pending(sync) != null) {
      throw StateError(
        'Project setup saved on this phone. Open New project to resume or discard it. ${error.toString().replaceFirst('Bad state: ', '')}',
      );
    }
    rethrow;
  }
}
