import 'package:flutter/material.dart';

/// Resolves bar width and spacing so a waveform occupies its full width.
@visibleForTesting
({double barWidth, double gap}) voiceNoteWaveformBarLayout({
  required double width,
  required int sampleCount,
}) {
  if (sampleCount <= 0 || width <= 0) return (barWidth: 0, gap: 0);
  const preferredGap = 2.0;
  final availableBarWidth =
      (width - (preferredGap * (sampleCount - 1))) / sampleCount;
  final barWidth = availableBarWidth.clamp(1.0, 3.0);
  if (sampleCount == 1) return (barWidth: barWidth, gap: 0);
  final gap = ((width - (barWidth * sampleCount)) / (sampleCount - 1)).clamp(
    0.0,
    double.infinity,
  );
  return (barWidth: barWidth, gap: gap);
}

/// Paints voice-note samples and optionally exposes seek semantics.
class VoiceNoteWaveform extends StatelessWidget {
  const VoiceNoteWaveform({
    super.key,
    required this.samples,
    this.progress = 0,
    this.onSeek,
    this.fadeEdges = false,
    this.height = 32,
    this.minimumBarHeight = 4,
    this.maximumBarHeight,
    this.colorOpacity = 1,
  });

  final List<double> samples;
  final double progress;
  final ValueChanged<double>? onSeek;
  final bool fadeEdges;
  final double height;
  final double minimumBarHeight;
  final double? maximumBarHeight;
  final double colorOpacity;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    Widget waveform = CustomPaint(
      key: const ValueKey('voice-note-waveform'),
      painter: _VoiceNoteWaveformPainter(
        samples: samples,
        progress: progress.clamp(0.0, 1.0),
        activeColor: colorScheme.primary.withValues(alpha: colorOpacity),
        inactiveColor: colorScheme.onSurfaceVariant.withValues(alpha: 0.46),
        minimumBarHeight: minimumBarHeight,
        maximumBarHeight: maximumBarHeight ?? height,
      ),
      size: Size(double.infinity, height),
    );
    if (fadeEdges) {
      waveform = ShaderMask(
        blendMode: BlendMode.dstIn,
        shaderCallback: (bounds) => const LinearGradient(
          colors: [
            Colors.transparent,
            Colors.white,
            Colors.white,
            Colors.transparent,
          ],
          stops: [0, 0.1, 0.9, 1],
        ).createShader(bounds),
        child: waveform,
      );
    }
    return LayoutBuilder(
      builder: (context, constraints) {
        void seek(double dx) {
          final width = constraints.maxWidth;
          if (onSeek != null && width > 0 && width.isFinite) {
            onSeek!((dx / width).clamp(0.0, 1.0));
          }
        }

        void adjust(double delta) =>
            onSeek?.call((progress.clamp(0.0, 1.0) + delta).clamp(0.0, 1.0));

        return Semantics(
          label: 'Voice note waveform',
          slider: onSeek != null,
          value: onSeek == null ? null : '${(progress * 100).round()} percent',
          increasedValue: onSeek == null
              ? null
              : '${((progress + 0.1).clamp(0.0, 1.0) * 100).round()} percent',
          decreasedValue: onSeek == null
              ? null
              : '${((progress - 0.1).clamp(0.0, 1.0) * 100).round()} percent',
          onIncrease: onSeek == null ? null : () => adjust(0.1),
          onDecrease: onSeek == null ? null : () => adjust(-0.1),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapDown: onSeek == null
                ? null
                : (details) => seek(details.localPosition.dx),
            onHorizontalDragStart: onSeek == null
                ? null
                : (details) => seek(details.localPosition.dx),
            onHorizontalDragUpdate: onSeek == null
                ? null
                : (details) => seek(details.localPosition.dx),
            child: SizedBox(
              height: height,
              width: double.infinity,
              child: waveform,
            ),
          ),
        );
      },
    );
  }
}

class _VoiceNoteWaveformPainter extends CustomPainter {
  const _VoiceNoteWaveformPainter({
    required this.samples,
    required this.progress,
    required this.activeColor,
    required this.inactiveColor,
    required this.minimumBarHeight,
    required this.maximumBarHeight,
  });

  final List<double> samples;
  final double progress;
  final Color activeColor;
  final Color inactiveColor;
  final double minimumBarHeight;
  final double maximumBarHeight;

  @override
  void paint(Canvas canvas, Size size) {
    if (samples.isEmpty || size.width <= 0 || size.height <= 0) return;
    final layout = voiceNoteWaveformBarLayout(
      width: size.width,
      sampleCount: samples.length,
    );
    final resolvedWidth = layout.barWidth;
    final gap = layout.gap;
    const originX = 0.0;
    final activeEdge = size.width * progress;
    final radius = Radius.circular(resolvedWidth / 2);

    void drawBars(Color color) {
      final paint = Paint()..color = color;
      for (var index = 0; index < samples.length; index++) {
        final maxBarHeight = maximumBarHeight.clamp(
          minimumBarHeight,
          size.height,
        );
        final barHeight =
            (minimumBarHeight +
                    (samples[index].clamp(0.0, 1.0) *
                        (maxBarHeight - minimumBarHeight)))
                .clamp(minimumBarHeight, maxBarHeight);
        final left = originX + index * (resolvedWidth + gap);
        final rect = Rect.fromLTWH(
          left,
          (size.height - barHeight) / 2,
          resolvedWidth,
          barHeight,
        );
        canvas.drawRRect(RRect.fromRectAndRadius(rect, radius), paint);
      }
    }

    drawBars(inactiveColor);
    if (activeEdge <= 0) return;
    canvas.save();
    canvas.clipRect(Rect.fromLTWH(0, 0, activeEdge, size.height));
    drawBars(activeColor);
    canvas.restore();
  }

  @override
  bool shouldRepaint(_VoiceNoteWaveformPainter oldDelegate) =>
      oldDelegate.samples != samples ||
      oldDelegate.progress != progress ||
      oldDelegate.activeColor != activeColor ||
      oldDelegate.inactiveColor != inactiveColor ||
      oldDelegate.minimumBarHeight != minimumBarHeight ||
      oldDelegate.maximumBarHeight != maximumBarHeight;
}
