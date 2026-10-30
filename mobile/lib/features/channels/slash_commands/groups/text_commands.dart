import '../slash_command.dart';

const shrugMarkdown = r'¯\\\_(ツ)\_/¯';

final List<SlashCommand> textSlashCommands = [
  SlashCommand(
    name: 'shrug',
    argumentHint: '[text]',
    description: 'Send your text followed by ¯\\_(ツ)_/¯',
    usage: '/shrug [text]',
    run: (ctx) => ctx.sendMessage(
      ctx.args.isEmpty ? shrugMarkdown : '${ctx.args} $shrugMarkdown',
      mentionPubkeys: ctx.mentionPubkeys,
    ),
  ),
];
