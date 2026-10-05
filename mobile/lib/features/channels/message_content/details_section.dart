part of '../message_content.dart';

/// Open sections, keyed by message id and [DetailsBlock.key]. Kept for the
/// app session so a reader's choice survives in-place edits of the message
/// (status boards) and scrolling it out of view; collapsed is the default.
final _openMessageSectionsProvider =
    NotifierProvider<_OpenMessageSections, Set<String>>(
      _OpenMessageSections.new,
    );

class _OpenMessageSections extends Notifier<Set<String>> {
  @override
  Set<String> build() => const {};

  void setOpen(String key, bool open) {
    state = open ? {...state, key} : ({...state}..remove(key));
  }
}

/// Renders [segments] from [splitDetailsBlocks]; [buildMarkdown] renders
/// every piece of markdown (text, titles, bodies) the way the message does.
class _MessageDetailsContent extends StatelessWidget {
  final List<DetailsSegment> segments;
  final String? messageId;

  /// Keys of the enclosing sections, so nested keys stay unique.
  final String keyPrefix;
  final Widget Function(String markdown) buildMarkdown;

  const _MessageDetailsContent({
    required this.segments,
    required this.messageId,
    required this.buildMarkdown,
    this.keyPrefix = '',
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final segment in segments)
          switch (segment) {
            DetailsText(:final text) =>
              text.trim().isEmpty
                  ? const SizedBox.shrink()
                  : buildMarkdown(text),
            DetailsBlock() => _MessageDetailsSection(
              block: segment,
              messageId: messageId,
              keyPrefix: keyPrefix,
              buildMarkdown: buildMarkdown,
            ),
          },
      ],
    );
  }
}

class _MessageDetailsSection extends HookConsumerWidget {
  final DetailsBlock block;
  final String? messageId;
  final String keyPrefix;
  final Widget Function(String markdown) buildMarkdown;

  const _MessageDetailsSection({
    required this.block,
    required this.messageId,
    required this.keyPrefix,
    required this.buildMarkdown,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sectionKey = '$keyPrefix/${block.key}';
    final storeKey = messageId == null ? null : '$messageId\u0000$sectionKey';
    final localOpen = useState(false);
    final open = storeKey == null
        ? localOpen.value
        : ref.watch(
            _openMessageSectionsProvider.select((s) => s.contains(storeKey)),
          );

    void toggle() {
      if (storeKey == null) {
        localOpen.value = !open;
      } else {
        ref
            .read(_openMessageSectionsProvider.notifier)
            .setOpen(storeKey, !open);
      }
    }

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.quarter),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Semantics(
            button: true,
            expanded: open,
            child: InkWell(
              onTap: toggle,
              borderRadius: BorderRadius.circular(Radii.sm),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: Grid.half),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Padding(
                      padding: const EdgeInsets.only(top: Grid.quarter),
                      child: Icon(
                        open
                            ? LucideIcons.chevronDown
                            : LucideIcons.chevronRight,
                        size: 16,
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                    const SizedBox(width: Grid.half),
                    Expanded(child: buildMarkdown(block.title)),
                  ],
                ),
              ),
            ),
          ),
          if (open)
            Padding(
              padding: const EdgeInsets.only(left: Grid.twelve + Grid.xxs),
              child: _MessageDetailsContent(
                segments: splitDetailsBlocks(block.body),
                messageId: messageId,
                keyPrefix: sectionKey,
                buildMarkdown: buildMarkdown,
              ),
            ),
        ],
      ),
    );
  }
}
