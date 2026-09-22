import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'canvas_board.dart';

class ChannelCanvasBoard extends HookWidget {
  final String channelName;
  final String? content;
  final String? errorMessage;
  final bool isLoading;
  final double topPadding;
  final Future<void> Function(String threadId) onOpenThread;

  const ChannelCanvasBoard({
    super.key,
    required this.channelName,
    required this.content,
    required this.errorMessage,
    required this.isLoading,
    required this.topPadding,
    required this.onOpenThread,
  });

  @override
  Widget build(BuildContext context) {
    final board = useMemoized(() => parseCanvasBoard(content ?? ''), [content]);
    final layout = useState(_BoardLayout.cards);

    return CustomScrollView(
      key: const ValueKey('channel-canvas-board'),
      slivers: [
        SliverPadding(
          padding: EdgeInsets.fromLTRB(
            Grid.gutter,
            topPadding + Grid.xs,
            Grid.gutter,
            Grid.xl,
          ),
          sliver: SliverList.list(
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          board.title ?? channelName,
                          style: context.textTheme.headlineSmall?.copyWith(
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                        if (board.introduction.isNotEmpty) ...[
                          const SizedBox(height: Grid.xxs),
                          GptMarkdown(
                            board.introduction,
                            style: context.textTheme.bodyMedium?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                  const SizedBox(width: Grid.xxs),
                  SegmentedButton<_BoardLayout>(
                    key: const ValueKey('channel-canvas-board-layout'),
                    segments: const [
                      ButtonSegment(
                        value: _BoardLayout.cards,
                        icon: Icon(LucideIcons.layoutGrid, size: 17),
                        tooltip: 'Cards',
                      ),
                      ButtonSegment(
                        value: _BoardLayout.kanban,
                        icon: Icon(LucideIcons.columns3, size: 17),
                        tooltip: 'Kanban',
                      ),
                    ],
                    selected: {layout.value},
                    showSelectedIcon: false,
                    onSelectionChanged: (selection) =>
                        layout.value = selection.first,
                  ),
                ],
              ),
              const SizedBox(height: Grid.xs),
              if (isLoading)
                const Padding(
                  padding: EdgeInsets.all(Grid.sm),
                  child: Center(
                    child: BuzzLoadingIndicator(
                      size: 40,
                      semanticLabel: 'Loading channel board',
                    ),
                  ),
                )
              else if (errorMessage case final error?)
                Container(
                  padding: const EdgeInsets.all(Grid.xs),
                  decoration: BoxDecoration(
                    color: context.colors.errorContainer,
                    borderRadius: BorderRadius.circular(16),
                  ),
                  child: Text(
                    error,
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: context.colors.onErrorContainer,
                    ),
                  ),
                )
              else if (board.cards.isEmpty)
                _EmptyBoard(channelName: channelName)
              else if (layout.value == _BoardLayout.cards)
                for (final card in board.cards) ...[
                  _CanvasCard(card: card, onOpenThread: onOpenThread),
                  const SizedBox(height: Grid.xxs),
                ]
              else
                for (final status in CanvasBoardCardStatus.values) ...[
                  _KanbanColumn(
                    status: status,
                    cards: board.cards
                        .where((card) => card.status == status)
                        .toList(),
                    onOpenThread: onOpenThread,
                  ),
                  const SizedBox(height: Grid.xs),
                ],
            ],
          ),
        ),
      ],
    );
  }
}

enum _BoardLayout { cards, kanban }

class _EmptyBoard extends StatelessWidget {
  final String channelName;

  const _EmptyBoard({required this.channelName});

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.all(Grid.sm),
    decoration: BoxDecoration(
      border: Border.all(color: context.colors.outlineVariant),
      borderRadius: BorderRadius.circular(16),
    ),
    child: Column(
      children: [
        Icon(LucideIcons.stickyNote, color: context.colors.onSurfaceVariant),
        const SizedBox(height: Grid.xxs),
        Text(
          '$channelName is ready for its first board card.',
          textAlign: TextAlign.center,
          style: context.textTheme.bodyMedium,
        ),
      ],
    ),
  );
}

class _KanbanColumn extends StatelessWidget {
  final CanvasBoardCardStatus status;
  final List<CanvasBoardCard> cards;
  final Future<void> Function(String threadId) onOpenThread;

  const _KanbanColumn({
    required this.status,
    required this.cards,
    required this.onOpenThread,
  });

  @override
  Widget build(BuildContext context) => Container(
    key: ValueKey('channel-canvas-kanban-${status.name}'),
    padding: const EdgeInsets.all(Grid.xxs),
    decoration: BoxDecoration(
      color: context.colors.surfaceContainerHighest.withValues(alpha: 0.35),
      borderRadius: BorderRadius.circular(16),
      border: Border.all(color: context.colors.outlineVariant),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(Grid.half),
          child: Row(
            children: [
              Icon(_statusIcon(status), size: 17),
              const SizedBox(width: Grid.half),
              Text(
                _statusLabel(status),
                style: context.textTheme.titleSmall?.copyWith(
                  fontWeight: FontWeight.w700,
                ),
              ),
              const Spacer(),
              Text('${cards.length}'),
            ],
          ),
        ),
        if (cards.isEmpty)
          Padding(
            padding: const EdgeInsets.all(Grid.xs),
            child: Text(
              'No cards',
              textAlign: TextAlign.center,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          )
        else
          for (final card in cards) ...[
            _CanvasCard(card: card, onOpenThread: onOpenThread),
            const SizedBox(height: Grid.xxs),
          ],
      ],
    ),
  );
}

class _CanvasCard extends StatelessWidget {
  final CanvasBoardCard card;
  final Future<void> Function(String threadId) onOpenThread;

  const _CanvasCard({required this.card, required this.onOpenThread});

  @override
  Widget build(BuildContext context) => Card(
    key: ValueKey('channel-canvas-card-${card.id}'),
    clipBehavior: Clip.antiAlias,
    child: Padding(
      padding: const EdgeInsets.all(Grid.xs),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(_typeIcon(card.type), size: 16),
              const SizedBox(width: Grid.half),
              Text(
                _typeLabel(card.type).toUpperCase(),
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                  fontWeight: FontWeight.w700,
                  letterSpacing: 1.1,
                ),
              ),
            ],
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            card.title,
            style: context.textTheme.titleMedium?.copyWith(
              fontWeight: FontWeight.w700,
            ),
          ),
          if (card.body.isNotEmpty) ...[
            const SizedBox(height: Grid.xxs),
            GptMarkdown(card.body, style: context.textTheme.bodyMedium),
          ],
          const SizedBox(height: Grid.xxs),
          Row(
            children: [
              Icon(
                _statusIcon(card.status),
                size: 15,
                color: context.colors.onSurfaceVariant,
              ),
              const SizedBox(width: Grid.half),
              Text(
                _statusLabel(card.status),
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
              const Spacer(),
              if (card.threadId case final threadId?)
                TextButton.icon(
                  key: ValueKey('channel-canvas-thread-${card.id}'),
                  onPressed: () => onOpenThread(threadId),
                  icon: const Icon(LucideIcons.messageCircle, size: 16),
                  label: const Text('Open thread'),
                ),
            ],
          ),
        ],
      ),
    ),
  );
}

String _typeLabel(CanvasBoardCardType type) => switch (type) {
  CanvasBoardCardType.agent => 'Agent',
  CanvasBoardCardType.artifact => 'Artifact',
  CanvasBoardCardType.conversation => 'Conversation',
  CanvasBoardCardType.decision => 'Decision',
  CanvasBoardCardType.note => 'Note',
  CanvasBoardCardType.person => 'Person',
  CanvasBoardCardType.project => 'Project',
  CanvasBoardCardType.task => 'Task',
};

String _statusLabel(CanvasBoardCardStatus status) => switch (status) {
  CanvasBoardCardStatus.backlog => 'Backlog',
  CanvasBoardCardStatus.doing => 'Doing',
  CanvasBoardCardStatus.done => 'Done',
};

IconData _typeIcon(CanvasBoardCardType type) => switch (type) {
  CanvasBoardCardType.agent => LucideIcons.bot,
  CanvasBoardCardType.artifact => LucideIcons.fileCheck2,
  CanvasBoardCardType.conversation => LucideIcons.messageCircle,
  CanvasBoardCardType.decision => LucideIcons.gavel,
  CanvasBoardCardType.note => LucideIcons.stickyNote,
  CanvasBoardCardType.person => LucideIcons.userRound,
  CanvasBoardCardType.project => LucideIcons.folderKanban,
  CanvasBoardCardType.task => LucideIcons.listTodo,
};

IconData _statusIcon(CanvasBoardCardStatus status) => switch (status) {
  CanvasBoardCardStatus.backlog => LucideIcons.circleDot,
  CanvasBoardCardStatus.doing => LucideIcons.sparkles,
  CanvasBoardCardStatus.done => LucideIcons.circleCheck,
};
