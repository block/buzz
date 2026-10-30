import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'modal_presentation.dart';
import 'ios_text_editor.dart';

/// Opens the shared native iOS text form, with a Flutter sheet fallback.
Future<void> showTextEditor({
  required BuildContext context,
  required String title,
  required String initialValue,
  required String hintText,
  required Future<void> Function(String value) onSave,
  bool multiline = false,
  String keyPrefix = 'text-editor',
  bool Function(Object error)? shouldRetryOnError,
}) async {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final sheetTheme = utilitySurfaceThemeData(Theme.of(context));
    try {
      await IosTextEditor.presentUntilSaved(
        title: title,
        initialValue: initialValue,
        placeholder: hintText,
        multiline: multiline,
        brightness: sheetTheme.brightness,
        pageBackgroundColor: sheetTheme.colorScheme.surface,
        containerBackgroundColor:
            sheetTheme.colorScheme.surfaceContainerHighest,
        onSave: onSave,
        shouldRetryOnError: shouldRetryOnError,
        canPresent: () =>
            context.mounted && (ModalRoute.of(context)?.isCurrent ?? true),
        onSaveError: () {
          if (context.mounted && (ModalRoute.of(context)?.isCurrent ?? true)) {
            _showSaveError(context);
          }
        },
      );
      return;
    } on MissingPluginException {
      // Previews and older builds retain the complete Flutter fallback.
    } on PlatformException {
      // A temporary native presentation failure should not block editing.
    }
  }

  if (!context.mounted) return;
  await showBuzzModalBottomSheet<void>(
    context: context,
    title: title,
    isScrollControlled: true,
    requestFocus: true,
    builder: (_) => _TextEditSheet(
      initialValue: initialValue,
      hintText: hintText,
      multiline: multiline,
      onSave: onSave,
      shouldRetryOnError: shouldRetryOnError,
      keyPrefix: keyPrefix,
    ),
  );
}

void _showSaveError(BuildContext context) {
  ScaffoldMessenger.of(context).showSnackBar(
    const SnackBar(content: Text("We couldn't save this change. Try again.")),
  );
}

class _TextEditSheet extends HookWidget {
  const _TextEditSheet({
    required this.initialValue,
    required this.hintText,
    required this.multiline,
    required this.onSave,
    this.shouldRetryOnError,
    required this.keyPrefix,
  });

  final String keyPrefix;
  final String initialValue;
  final String hintText;
  final bool multiline;
  final Future<void> Function(String value) onSave;
  final bool Function(Object error)? shouldRetryOnError;

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
      } catch (saveError) {
        if (!context.mounted) return;
        if (shouldRetryOnError?.call(saveError) == false) {
          Navigator.of(context).pop();
          return;
        }
        error.value = "We couldn't save this change. Try again.";
      } finally {
        if (context.mounted) isSaving.value = false;
      }
    }

    return SafeArea(
      top: false,
      child: SingleChildScrollView(
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
              key: ValueKey('$keyPrefix-input'),
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
              key: ValueKey('$keyPrefix-save'),
              onPressed: hasChanges && !isSaving.value ? save : null,
              child: Text(isSaving.value ? 'Saving…' : 'Save'),
            ),
          ],
        ),
      ),
    );
  }
}
