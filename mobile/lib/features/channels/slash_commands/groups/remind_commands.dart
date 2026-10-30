import 'package:intl/intl.dart';

import '../../../../shared/reminders/remind_time_parser.dart';
import '../../../../shared/reminders/reminder_service.dart';
import '../../../../shared/slash_commands/slash_errors.dart';
import '../slash_command.dart';

final _dueFormat = DateFormat("EEE, MMM d 'at' h:mm a");

final List<SlashCommand> remindSlashCommands = [
  SlashCommand(
    name: 'remind',
    argumentHint: 'me <what> <when>',
    description: 'Set a reminder for yourself',
    usage: '/remind me <what> <when>',
    run: (ctx) async {
      final RemindParsed reminder = switch (parseRemindCommand(
        ctx.args,
        now: DateTime.now(),
      )) {
        RemindRejected(:final error) => throw SlashUsageError(error.message),
        final RemindParsed parsed => parsed,
      };
      final service = ctx.ref.read(reminderServiceProvider);
      if (service == null) {
        throw const SlashCommandFailure('Sign in to set reminders');
      }
      await service.createNoteReminder(
        note: reminder.note,
        notBefore: reminder.dueAt.millisecondsSinceEpoch ~/ 1000,
      );
      ctx.showMessage('Reminder set for ${_dueFormat.format(reminder.dueAt)}');
    },
  ),
];
