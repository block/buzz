part of '../message_content.dart';

/// Open sections, keyed by message id and [DetailsBlock.key]. Kept for the
/// app session so a reader's choice survives in-place edits of the message
/// (status boards) and scrolling it out of view; collapsed is the default.
/// Rebuilt empty on a community switch, and capped so the oldest choices are
/// forgotten first.
final _openMessageSectionsProvider =
    NotifierProvider<_OpenMessageSections, Set<String>>(
      _OpenMessageSections.new,
    );

const _openMessageSectionsLimit = 500;

class _OpenMessageSections extends Notifier<Set<String>> {
  @override
  Set<String> build() {
    ref.watch(relayConfigProvider);
    return const {};
  }

  void setOpen(String key, bool open) {
    final next = {...state}..remove(key);
    if (open) next.add(key);
    while (next.length > _openMessageSectionsLimit) {
      next.remove(next.first);
    }
    state = next;
  }
}

/// Renders [segments] from [splitDetailsBlocks]; [buildMarkdown] renders
/// text and bodies the way the message does.
class _MessageDetailsContent extends StatelessWidget {
  final List<DetailsSegment> segments;
  final String? messageId;
  final TextStyle? titleStyle;

  /// Keys of the enclosing sections, so nested keys stay unique.
  final String keyPrefix;

  /// Nesting level of [segments], bounding how deep their bodies may nest.
  final int depth;
  final Widget Function(String markdown, {TextStyle? textStyle, bool plain})
  buildMarkdown;

  const _MessageDetailsContent({
    required this.segments,
    required this.messageId,
    required this.titleStyle,
    required this.buildMarkdown,
    this.keyPrefix = '',
    this.depth = 0,
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
              titleStyle: titleStyle,
              keyPrefix: keyPrefix,
              depth: depth,
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
  final TextStyle? titleStyle;
  final String keyPrefix;
  final int depth;
  final Widget Function(String markdown, {TextStyle? textStyle, bool plain})
  buildMarkdown;

  const _MessageDetailsSection({
    required this.block,
    required this.messageId,
    required this.titleStyle,
    required this.keyPrefix,
    required this.depth,
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
            header: block.level > 0,
            expanded: open,
            label: block.title,
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
                    // Formatting only: the title's own links and mentions
                    // stay inert, so the toggle is the single control.
                    Expanded(
                      child: IgnorePointer(
                        child: ExcludeSemantics(
                          child: buildMarkdown(
                            block.level > 0
                                ? '${'#' * block.level} ${block.titleMarkdown}'
                                : block.titleMarkdown,
                            // Mentions and emoji as text, as on desktop.
                            plain: true,
                            textStyle: block.level > 0
                                ? null
                                : titleStyle?.copyWith(
                                    fontWeight: FontWeight.w600,
                                  ),
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
          if (open)
            Padding(
              padding: const EdgeInsets.only(left: Grid.twelve + Grid.xxs),
              child: _MessageDetailsContent(
                segments: splitDetailsBlocks(block.body, depth: depth + 1),
                messageId: messageId,
                titleStyle: titleStyle,
                keyPrefix: sectionKey,
                depth: depth + 1,
                buildMarkdown: buildMarkdown,
              ),
            ),
        ],
      ),
    );
  }
}
