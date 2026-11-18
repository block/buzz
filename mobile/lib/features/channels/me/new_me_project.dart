import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';
import '../../../shared/me/me_sync.dart';
import '../../../shared/me/me_project_creation.dart';
import '../../../shared/me/me_recovery.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/modal_presentation.dart';

/// Creates or resumes a workspace and returns its stable ID.
Future<String?> createMeProject(
  BuildContext context,
  WidgetRef ref, {
  bool workspace = false,
  String? channelId,
}) async {
  final noun = workspace ? 'workspace' : 'project';
  final sync = ref.read(meSyncProvider);
  var intent = MeProjectCreation.pending(sync);
  if (intent != null) {
    final reviewed = intent;
    final resume = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Unfinished project'),
        content: Text(
          'Resume “${reviewed.name}”, or discard its setup to start another project. Resuming also finishes the original chat placement, if any. Any project already sent or awaiting confirmation is kept.',
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
    if (resume == null || !context.mounted) return null;
    if (!resume) {
      await reviewed.discard(sync);
      intent = null;
    }
  }
  if (!context.mounted) return null;
  if (intent == null) {
    final name = await _name(context, 'New $noun', noun: noun);
    if (name == null) return null;
    intent = MeProjectCreation(const Uuid().v4(), name, channelId: channelId);
  }
  while (context.mounted) {
    try {
      final current = ref.read(meSyncProvider);
      if (current.community != sync.community ||
          current.pubkey != sync.pubkey) {
        throw StateError('Your account or community changed. Open Solo again.');
      }
      await intent!.complete(current);
      return intent.channelId == channelId ? intent.id : null;
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
      if (retry != true || !context.mounted) return null;
      final name = await _name(
        context,
        'New $noun',
        noun: noun,
        initialValue: intent!.name,
      );
      if (name == null) return null;
      intent = MeProjectCreation(intent.id, name, channelId: intent.channelId);
    }
  }
  return null;
}

Future<String?> _name(
  BuildContext context,
  String title, {
  String initialValue = '',
  String noun = 'project',
}) async {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final theme = utilitySurfaceThemeData(Theme.of(context));
    try {
      return await const MethodChannel(
        'buzz/profile_text_editor',
      ).invokeMethod<String>('presentProject', {
        'title': title,
        'initialValue': initialValue,
        'placeholder': '${noun[0].toUpperCase()}${noun.substring(1)} name',
        'multiline': false,
        'brightness': theme.brightness.name,
        'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
        'containerBackgroundArgb': theme.colorScheme.surfaceContainerHighest
            .toARGB32(),
        'containerCornerRadius': Radii.container,
      });
    } on MissingPluginException {
      // Use the shared sheet when the native editor is unavailable.
    }
  }
  if (!context.mounted) return null;
  return showBuzzModalBottomSheet<String>(
    context: context,
    title: title,
    isScrollControlled: true,
    requestFocus: true,
    builder: (_) => _NewMeProject(initialValue: initialValue, noun: noun),
  );
}

class _NewMeProject extends HookConsumerWidget {
  final String initialValue;
  final String noun;
  const _NewMeProject({this.initialValue = '', this.noun = 'project'});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController(text: initialValue);
    useListenable(controller);
    final name = controller.text.trim();
    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.md,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.gutter,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            controller: controller,
            autofocus: true,
            maxLength: 120,
            decoration: InputDecoration(
              labelText: '${noun[0].toUpperCase()}${noun.substring(1)} name',
            ),
          ),
          const SizedBox(height: Grid.md),
          FilledButton(
            onPressed: name.isEmpty ? null : () => Navigator.pop(context, name),
            child: Text('Create $noun'),
          ),
        ],
      ),
    );
  }
}
