import 'dart:async';

import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import '../widgets/app_list.dart';
import '../widgets/feedback_snackbar.dart';
import '../widgets/modal_presentation.dart';
import '../widgets/sheet_action_section.dart';
import 'reminder_service.dart';
import 'reminder_time_choices.dart';
import 'reminder_time_presets.dart';

/// The saved parts of a pending reminder that [showEditReminderSheet] keeps
/// as they are while it changes the note.
typedef EditableReminder = ({
  String dTag,
  int notBefore,
  ReminderTarget? target,
  String? note,
});

/// Bottom sheet for "New reminder": required reminder text first, then when,
/// publishing the same kind-30300 event desktop writes for a written
/// reminder (a `note` and no `target`). [onSaved] runs after the relay
/// accepts it.
void showNewReminderSheet({
  required BuildContext context,
  required WidgetRef ref,
  VoidCallback? onSaved,
}) {
  final service = _serviceOrNotice(context, ref);
  if (service == null) return;
  final messenger = ScaffoldMessenger.of(context);
  _showSheet(
    context: context,
    title: 'New reminder',
    sheet: _ReminderTextSheet(
      fieldLabel: 'Reminder text',
      textRequired: true,
      withTime: true,
      submitLabel: 'Set reminder',
      save: (text, notBefore) =>
          service.createTextReminder(text: text, notBefore: notBefore!),
      onSaved: () {
        onSaved?.call();
        _confirm(messenger, 'Reminder set');
      },
    ),
  );
}

/// Bottom sheet for "Edit reminder": only the note changes. A message
/// reminder's note is optional and may be cleared; a written reminder's text
/// is its whole content and stays required. The reminder keeps its `d` tag,
/// target and time. [onSaved] runs after the relay accepts the edit.
void showEditReminderSheet({
  required BuildContext context,
  required WidgetRef ref,
  required EditableReminder reminder,
  VoidCallback? onSaved,
}) {
  final service = _serviceOrNotice(context, ref);
  if (service == null) return;
  final messenger = ScaffoldMessenger.of(context);
  final written = reminder.target == null;
  _showSheet(
    context: context,
    title: 'Edit reminder',
    sheet: _ReminderTextSheet(
      fieldLabel: written ? 'Reminder text' : 'Note (optional)',
      textRequired: written,
      withTime: false,
      initialText: reminder.note ?? '',
      submitLabel: 'Save',
      save: (text, _) => service.updateReminderNote(
        dTag: reminder.dTag,
        notBefore: reminder.notBefore,
        target: reminder.target,
        note: text,
      ),
      onSaved: () {
        onSaved?.call();
        _confirm(messenger, 'Reminder updated');
      },
    ),
  );
}

ReminderService? _serviceOrNotice(BuildContext context, WidgetRef ref) {
  final service = ref.read(reminderServiceProvider);
  if (service == null) {
    ScaffoldMessenger.of(context).showFeedbackSnackBar(
      const SnackBar(content: Text('Sign in to set reminders')),
      kind: ToastKind.generic,
    );
  }
  return service;
}

void _confirm(ScaffoldMessengerState messenger, String message) {
  messenger.showFeedbackSnackBar(
    SnackBar(content: Text(message)),
    kind: ToastKind.success,
  );
}

void _showSheet({
  required BuildContext context,
  required String title,
  required Widget sheet,
}) {
  showBuzzModalBottomSheet<void>(
    context: context,
    title: title,
    isScrollControlled: true,
    showDragHandle: true,
    requestFocus: true,
    builder: (_) => sheet,
  );
}

const _saveFailedCopy = 'The reminder could not be saved. Try again.';

class _ReminderTextSheet extends HookWidget {
  const _ReminderTextSheet({
    required this.fieldLabel,
    required this.textRequired,
    required this.withTime,
    required this.submitLabel,
    required this.save,
    required this.onSaved,
    this.initialText = '',
  });

  final String fieldLabel;
  final bool textRequired;

  /// Whether to offer when the reminder is due. Off for an edit, which keeps
  /// the saved time.
  final bool withTime;
  final String submitLabel;
  final String initialText;

  /// Writes the reminder: the trimmed text, and the due time when
  /// [withTime]. Throws on failure.
  final Future<void> Function(String text, int? notBefore) save;
  final VoidCallback onSaved;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialText);
    useListenable(controller);
    final when = useState<ReminderTimePreset>(reminderTimePresets.first);
    final saving = useState(false);
    final failed = useState(false);

    final text = controller.text.trim();
    // A new reminder saves once it has its text; an edit also needs a change.
    final canSave =
        !saving.value &&
        (!textRequired || text.isNotEmpty) &&
        (withTime || text != initialText.trim());

    Future<void> chooseTime() async {
      FocusScope.of(context).unfocus();
      final choice = await showReminderTimeChoicesSheet(context);
      if (choice != null) when.value = choice;
    }

    Future<void> submit() async {
      final navigator = Navigator.of(context);
      saving.value = true;
      failed.value = false;
      try {
        await save(text, withTime ? when.value.getTimestamp() : null);
      } catch (error) {
        // Stable user-facing copy; the relay/crypto detail goes to the log.
        debugPrint('[Reminder] save failed: $error');
        if (!context.mounted) return;
        saving.value = false;
        failed.value = true;
        return;
      }
      if (navigator.mounted) navigator.pop();
      onSaved();
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xxs,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.gutter,
      ),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              key: const ValueKey('reminder-text-field'),
              controller: controller,
              enabled: !saving.value,
              autofocus: true,
              minLines: 3,
              maxLines: 8,
              keyboardType: TextInputType.multiline,
              textCapitalization: TextCapitalization.sentences,
              inputFormatters: [
                LengthLimitingTextInputFormatter(maxReminderNoteLength),
              ],
              decoration: InputDecoration(
                labelText: fieldLabel,
                hintText: 'What do you want to follow up on?',
                alignLabelWithHint: true,
              ),
            ),
            if (withTime) ...[
              const SizedBox(height: Grid.xs),
              SheetActionSection(
                children: [
                  AppListRow(
                    icon: BuzzIcons.clock,
                    title: 'When',
                    value: when.value.label,
                    trailing: const Icon(BuzzIcons.chevronDown, size: 18),
                    onTap: saving.value ? null : chooseTime,
                  ),
                ],
              ),
            ],
            if (failed.value) ...[
              const SizedBox(height: Grid.xs),
              Semantics(
                liveRegion: true,
                child: Text(
                  _saveFailedCopy,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
            ],
            const SizedBox(height: Grid.md),
            FilledButton(
              onPressed: canSave ? () => unawaited(submit()) : null,
              child: Text(submitLabel),
            ),
          ],
        ),
      ),
    );
  }
}
