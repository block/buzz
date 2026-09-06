import 'dart:math' as math;
import 'dart:ui' show FontFeature;

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import 'agent_usage_detail_sheet.dart';
import 'agent_usage_models.dart';
import 'agent_usage_subscription.dart';

/// Compact, permanent usage ring drawn around an agent's avatar.
///
/// Shows the agent's most urgent known usage fraction (the highest
/// provider-account quota window, falling back to context-window usage) as a
/// thin colored arc. Tapping opens a detail sheet with the full breakdown.
/// Renders nothing extra (just the plain [child]) when no usage data has
/// ever been observed and the subscription is otherwise healthy — an agent
/// that hasn't reported usage yet is not an error state.
class AgentUsageIndicator extends ConsumerWidget {
  final String agentPubkey;
  final String agentLabel;
  final Widget child;

  /// Diameter of [child] (e.g. an avatar). The ring is drawn just outside it.
  final double childDiameter;

  const AgentUsageIndicator({
    super.key,
    required this.agentPubkey,
    required this.agentLabel,
    required this.child,
    this.childDiameter = 40,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final normalizedAgent = agentPubkey.toLowerCase();
    final relayState = ref.watch(agentUsageRelayProvider);
    final snapshot = relayState.snapshotsByAgent[normalizedAgent];
    final subscriptionErrored =
        relayState.connection == AgentUsageConnectionState.error;
    final status = agentUsageStatusFor(
      snapshot,
      subscriptionErrored: subscriptionErrored,
    );
    final fraction = primaryUsageFraction(snapshot);

    final ringDiameter = childDiameter + Grid.half * 2;
    final color = _colorFor(context, status, fraction);

    return Semantics(
      button: true,
      label: _semanticsLabel(status, fraction),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => showAgentUsageDetailSheet(
          context: context,
          agentLabel: agentLabel,
          snapshot: snapshot,
          status: status,
        ),
        child: ExcludeSemantics(
          child: SizedBox(
            width: ringDiameter,
            height: ringDiameter,
            child: CustomPaint(
              painter: status == AgentUsageStatus.unknown
                  ? null
                  : _UsageRingPainter(
                      fraction: fraction,
                      color: color,
                      dashed: status == AgentUsageStatus.stale,
                      isError: status == AgentUsageStatus.error,
                    ),
              child: Center(
                child: SizedBox(
                  width: childDiameter,
                  height: childDiameter,
                  child: child,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  static Color _colorFor(
    BuildContext context,
    AgentUsageStatus status,
    double? fraction,
  ) {
    if (status == AgentUsageStatus.error) return context.colors.error;
    if (fraction == null) return context.colors.onSurfaceVariant;
    if (fraction >= 0.9) return context.colors.error;
    if (fraction >= 0.7) return context.appColors.warning;
    return context.colors.onSurfaceVariant;
  }

  static String _semanticsLabel(AgentUsageStatus status, double? fraction) {
    switch (status) {
      case AgentUsageStatus.unknown:
        return 'Agent usage: no data yet';
      case AgentUsageStatus.error:
        return 'Agent usage: unavailable';
      case AgentUsageStatus.stale:
        final pct = fraction != null ? '${(fraction * 100).round()}%, ' : '';
        return 'Agent usage: ${pct}data may be out of date';
      case AgentUsageStatus.fresh:
        final pct = fraction != null
            ? '${(fraction * 100).round()}% used'
            : 'active';
        return 'Agent usage: $pct';
    }
  }
}

/// Small visible percentage placed next to an agent name. The ring remains the
/// tap target; this label deliberately renders nothing for unknown usage rather
/// than inventing a zero.
class AgentUsagePercentText extends ConsumerWidget {
  final String agentPubkey;

  const AgentUsagePercentText({super.key, required this.agentPubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final relayState = ref.watch(agentUsageRelayProvider);
    final snapshot = relayState.snapshotsByAgent[agentPubkey.toLowerCase()];
    final fraction = primaryUsageFraction(snapshot);
    if (fraction == null) return const SizedBox.shrink();

    final status = agentUsageStatusFor(
      snapshot,
      subscriptionErrored:
          relayState.connection == AgentUsageConnectionState.error,
    );
    final percent = (fraction * 100).round();
    return Text(
      '$percent%',
      style: context.textTheme.labelSmall?.copyWith(
        color: AgentUsageIndicator._colorFor(context, status, fraction),
        fontFeatures: const [FontFeature.tabularFigures()],
        fontWeight: FontWeight.w600,
      ),
    );
  }
}

/// Paints a thin progress arc around the child avatar. A solid arc for fresh
/// data, a dashed arc when the underlying snapshot is [AgentUsageStatus.stale],
/// and a full dim ring with no progress readout when [isError] (no data and
/// the subscription itself is unhealthy).
class _UsageRingPainter extends CustomPainter {
  final double? fraction;
  final Color color;
  final bool dashed;
  final bool isError;

  static const _strokeWidth = 2.5;

  const _UsageRingPainter({
    required this.fraction,
    required this.color,
    required this.dashed,
    required this.isError,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final center = Offset(size.width / 2, size.height / 2);
    final radius = (math.min(size.width, size.height) - _strokeWidth) / 2;
    final rect = Rect.fromCircle(center: center, radius: radius);
    final backgroundPaint = Paint()
      ..color = color.withValues(alpha: 0.2)
      ..style = PaintingStyle.stroke
      ..strokeWidth = _strokeWidth;
    canvas.drawArc(rect, 0, 2 * math.pi, false, backgroundPaint);

    if (isError) {
      return;
    }

    final sweep = 2 * math.pi * (fraction ?? 1.0);
    final foregroundPaint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = _strokeWidth
      ..strokeCap = StrokeCap.round;

    if (!dashed) {
      canvas.drawArc(rect, -math.pi / 2, sweep, false, foregroundPaint);
      return;
    }

    const dashLength = 0.12; // radians of arc drawn per dash
    const gapLength = 0.08; // radians of arc skipped per dash
    var drawn = 0.0;
    var start = -math.pi / 2;
    while (drawn < sweep) {
      final segment = math.min(dashLength, sweep - drawn);
      canvas.drawArc(rect, start, segment, false, foregroundPaint);
      start += dashLength + gapLength;
      drawn += dashLength + gapLength;
    }
  }

  @override
  bool shouldRepaint(covariant _UsageRingPainter oldDelegate) {
    return oldDelegate.fraction != fraction ||
        oldDelegate.color != color ||
        oldDelegate.dashed != dashed ||
        oldDelegate.isError != isError;
  }
}
