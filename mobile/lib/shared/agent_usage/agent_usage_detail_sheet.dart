import 'package:flutter/material.dart';

import '../widgets/modal_presentation.dart';
import '../theme/theme.dart';
import 'agent_usage_models.dart';

/// Opens the detail breakdown for one agent's usage snapshot.
Future<void> showAgentUsageDetailSheet({
  required BuildContext context,
  required String agentLabel,
  required AgentUsageSnapshot? snapshot,
  required AgentUsageStatus status,
}) {
  return showBuzzModalBottomSheet<void>(
    context: context,
    title: agentLabel,
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xxs,
          Grid.gutter,
          Grid.sm,
        ),
        child: SingleChildScrollView(
          child: _AgentUsageDetailBody(snapshot: snapshot, status: status),
        ),
      ),
    ),
  );
}

class _AgentUsageDetailBody extends StatelessWidget {
  final AgentUsageSnapshot? snapshot;
  final AgentUsageStatus status;

  const _AgentUsageDetailBody({required this.snapshot, required this.status});

  @override
  Widget build(BuildContext context) {
    final snapshot = this.snapshot;
    if (snapshot == null) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.md),
        child: Text(
          status == AgentUsageStatus.error
              ? 'Usage data unavailable — the live subscription is not connected.'
              : 'No usage data reported yet.',
          style: context.textTheme.bodyMedium?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
      );
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _StatusLine(snapshot: snapshot, status: status),
        const SizedBox(height: Grid.xxs),
        if (snapshot.contextUsedTokens != null &&
            snapshot.contextLimitTokens != null)
          _UsageRow(
            label: 'Context window',
            valueText:
                '${_formatTokens(snapshot.contextUsedTokens!)} / ${_formatTokens(snapshot.contextLimitTokens!)}',
            fraction: snapshot.contextUsedFraction,
          ),
        for (final window in snapshot.accountUsageWindows)
          _UsageRow(
            label: window.label,
            valueText:
                '${window.usedPercent.toStringAsFixed(0)}%'
                '${window.resetAt != null ? ' · resets ${_formatRelative(window.resetAt!)}' : ''}',
            fraction: (window.usedPercent / 100).clamp(0.0, 1.0),
          ),
        if (snapshot.cumulative?.totalTokens != null ||
            snapshot.cumulative?.costUsd != null) ...[
          const SizedBox(height: Grid.xxs),
          Text(
            'Session total: '
            '${snapshot.cumulative?.totalTokens != null ? '${_formatTokens(snapshot.cumulative!.totalTokens!)} tokens' : 'unknown'}'
            '${snapshot.cumulative?.costUsd != null ? ' · \$${snapshot.cumulative!.costUsd!.toStringAsFixed(2)}' : ''}',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ],
    );
  }
}

class _StatusLine extends StatelessWidget {
  final AgentUsageSnapshot snapshot;
  final AgentUsageStatus status;

  const _StatusLine({required this.snapshot, required this.status});

  @override
  Widget build(BuildContext context) {
    final harnessLine = snapshot.model != null
        ? '${snapshot.harness} · ${snapshot.model}'
        : snapshot.harness;
    final asOf = 'as of ${_formatRelative(snapshot.lastEventAt)}';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          harnessLine,
          style: context.textTheme.bodyMedium?.copyWith(
            fontWeight: FontWeight.w600,
          ),
        ),
        Text(
          status == AgentUsageStatus.stale
              ? '$asOf · may be out of date'
              : asOf,
          style: context.textTheme.bodySmall?.copyWith(
            color: status == AgentUsageStatus.stale
                ? context.appColors.warning
                : context.colors.onSurfaceVariant,
          ),
        ),
      ],
    );
  }
}

class _UsageRow extends StatelessWidget {
  final String label;
  final String valueText;
  final double? fraction;

  const _UsageRow({
    required this.label,
    required this.valueText,
    required this.fraction,
  });

  @override
  Widget build(BuildContext context) {
    final color = fraction == null
        ? context.colors.onSurfaceVariant
        : fraction! >= 0.9
        ? context.colors.error
        : fraction! >= 0.7
        ? context.appColors.warning
        : context.colors.onSurfaceVariant;

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.quarter),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  label,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: context.textTheme.bodyMedium,
                ),
              ),
              const SizedBox(width: Grid.xxs),
              Flexible(
                child: Text(
                  valueText,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.end,
                  style: context.textTheme.bodyMedium?.copyWith(color: color),
                ),
              ),
            ],
          ),
          if (fraction != null) ...[
            const SizedBox(height: Grid.quarter),
            ClipRRect(
              borderRadius: BorderRadius.circular(Radii.full),
              child: LinearProgressIndicator(
                value: fraction,
                minHeight: 4,
                backgroundColor: color.withValues(alpha: 0.15),
                valueColor: AlwaysStoppedAnimation<Color>(color),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

String _formatTokens(int tokens) {
  if (tokens >= 1000000) {
    return '${(tokens / 1000000).toStringAsFixed(1)}M';
  }
  if (tokens >= 1000) {
    return '${(tokens / 1000).toStringAsFixed(1)}k';
  }
  return tokens.toString();
}

String _formatRelative(DateTime time) {
  final now = DateTime.now();
  final isFuture = time.isAfter(now);
  final diff = (isFuture ? time.difference(now) : now.difference(time));
  final String magnitude;
  if (diff.inMinutes < 1) {
    magnitude = 'moment';
  } else if (diff.inMinutes < 60) {
    magnitude = '${diff.inMinutes}m';
  } else if (diff.inHours < 24) {
    magnitude = '${diff.inHours}h';
  } else {
    magnitude = '${diff.inDays}d';
  }
  if (magnitude == 'moment') {
    return isFuture ? 'in a moment' : 'just now';
  }
  return isFuture ? 'in $magnitude' : '$magnitude ago';
}
