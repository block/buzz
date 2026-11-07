part of '../compose_bar.dart';

class _SlashSuggestions extends StatelessWidget {
  final List<SlashCommand> suggestions;
  final void Function(SlashCommand) onSelect;

  const _SlashSuggestions({required this.suggestions, required this.onSelect});

  @override
  Widget build(BuildContext context) {
    return Material(
      key: const ValueKey('slash-suggestions-popover'),
      type: MaterialType.card,
      color: appPopoverColor(context),
      surfaceTintColor: Colors.transparent,
      elevation: appPopoverElevation,
      shadowColor: appPopoverShadowColor(context),
      shape: appPopoverShape(context),
      clipBehavior: Clip.hardEdge,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 240),
        child: ListView.builder(
          shrinkWrap: true,
          padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
          itemCount: suggestions.length,
          itemBuilder: (context, index) {
            final command = suggestions[index];
            return ListTile(
              dense: true,
              visualDensity: VisualDensity.compact,
              title: Text.rich(
                TextSpan(
                  text: '/${command.name}',
                  children: [
                    if (command.argumentHint.isNotEmpty)
                      TextSpan(
                        text: ' ${command.argumentHint}',
                        style: TextStyle(
                          color: context.colors.onSurfaceVariant,
                          fontWeight: FontWeight.w400,
                        ),
                      ),
                  ],
                ),
                style: context.textTheme.titleSmall,
              ),
              subtitle: Text(
                command.description,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
                overflow: TextOverflow.ellipsis,
              ),
              onTap: () => _runComposerAction(() => onSelect(command)),
            );
          },
        ),
      ),
    );
  }
}
