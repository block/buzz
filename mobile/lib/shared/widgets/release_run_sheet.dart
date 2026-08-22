import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:url_launcher/url_launcher.dart';

import '../deeplink/deep_link.dart';
import '../theme/theme.dart';
import 'modal_presentation.dart';

enum _ReleaseRunState { loading, empty, failed, ready }

_ReleaseRunState _releaseRunState(ReleaseRunDeepLink run) {
  final status = run.status.toLowerCase();
  if (RegExp(r'running|pending|queued|started').hasMatch(status)) {
    return _ReleaseRunState.loading;
  }
  if (RegExp(r'failed|error|critical').hasMatch(status) && run.tracks.isEmpty) {
    return _ReleaseRunState.failed;
  }
  return run.tracks.isEmpty ? _ReleaseRunState.empty : _ReleaseRunState.ready;
}

Future<void> showReleaseRunSheet(BuildContext context, ReleaseRunDeepLink run) {
  return showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    useRootNavigator: true,
    backgroundColor: Colors.transparent,
    barrierColor: Colors.black.withValues(alpha: 0.28),
    showCloseButton: false,
    builder: (_) => _ReleaseRunSheet(run: run),
  );
}

class _ReleaseRunSheet extends StatelessWidget {
  const _ReleaseRunSheet({required this.run});

  final ReleaseRunDeepLink run;

  @override
  Widget build(BuildContext context) {
    final highContrast = MediaQuery.highContrastOf(context);
    final surfaceColor = context.colors.surface.withValues(
      alpha: highContrast ? 1 : 0.86,
    );
    final content = DecoratedBox(
      decoration: BoxDecoration(
        color: surfaceColor,
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.72),
        ),
        borderRadius: const BorderRadius.vertical(
          top: Radius.circular(Radii.dialog),
        ),
        boxShadow: const [
          BoxShadow(
            color: Color(0x4D000000),
            blurRadius: 54,
            spreadRadius: -24,
            offset: Offset(0, -10),
          ),
        ],
      ),
      child: SafeArea(
        top: false,
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * 0.82,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              _ReleaseRunHeader(run: run),
              Flexible(child: _ReleaseRunBody(run: run)),
              _ReleaseRunFooter(sourceHealth: run.sourceHealth),
            ],
          ),
        ),
      ),
    );

    return ClipRRect(
      borderRadius: const BorderRadius.vertical(
        top: Radius.circular(Radii.dialog),
      ),
      child: highContrast
          ? content
          : BackdropFilter(
              filter: ImageFilter.blur(sigmaX: 24, sigmaY: 24),
              child: content,
            ),
    );
  }
}

class _ReleaseRunHeader extends StatelessWidget {
  const _ReleaseRunHeader({required this.run});

  final ReleaseRunDeepLink run;

  @override
  Widget build(BuildContext context) {
    final releaseLabel = run.released == 0
        ? 'Release run'
        : 'Released · ${run.released} ${run.released == 1 ? 'track' : 'tracks'}';
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(
            color: context.colors.outlineVariant.withValues(alpha: 0.58),
          ),
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.xs,
          Grid.twelve,
          Grid.xxs,
          Grid.twelve,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: const Color(0xFFE24A59),
                borderRadius: BorderRadius.circular(Radii.md),
                boxShadow: const [
                  BoxShadow(
                    color: Color(0x4DE24A59),
                    blurRadius: 18,
                    spreadRadius: -9,
                    offset: Offset(0, 8),
                  ),
                ],
              ),
              child: const Icon(
                LucideIcons.disc3,
                color: Colors.white,
                size: 17,
              ),
            ),
            const SizedBox(width: Grid.twelve),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    releaseLabel,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.titleMedium?.copyWith(
                      color: context.colors.onSurface,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: Grid.xxs),
                  Row(
                    children: [
                      Icon(
                        LucideIcons.clock3,
                        size: 12,
                        color: context.colors.onSurfaceVariant,
                      ),
                      const SizedBox(width: Grid.xxs),
                      Expanded(
                        child: Text(
                          '${DateFormat('MMM d · h:mm a').format(run.finishedAt.toLocal())} · ${run.runName}',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.labelSmall?.copyWith(
                            color: context.colors.onSurfaceVariant,
                          ),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            ),
            IconButton(
              tooltip: 'Close release preview',
              onPressed: () => Navigator.of(context).pop(),
              icon: const Icon(LucideIcons.x, size: 18),
            ),
          ],
        ),
      ),
    );
  }
}

class _ReleaseRunBody extends StatelessWidget {
  const _ReleaseRunBody({required this.run});

  final ReleaseRunDeepLink run;

  @override
  Widget build(BuildContext context) {
    final state = _releaseRunState(run);
    if (state == _ReleaseRunState.ready) {
      return ListView.separated(
        shrinkWrap: true,
        padding: EdgeInsets.zero,
        itemCount: run.tracks.length,
        separatorBuilder: (_, _) => Divider(
          height: 1,
          color: context.colors.outlineVariant.withValues(alpha: 0.52),
        ),
        itemBuilder: (_, index) => _ReleaseTrackRow(track: run.tracks[index]),
      );
    }

    final presentation = switch (state) {
      _ReleaseRunState.loading => (
        icon: LucideIcons.refreshCw,
        title: 'Release check in progress',
        description:
            'This preview will be populated by the completed run report.',
      ),
      _ReleaseRunState.failed => (
        icon: LucideIcons.circleAlert,
        title: 'Run needs attention',
        description: run.sourceHealth,
      ),
      _ => (
        icon: LucideIcons.disc3,
        title: 'No tracks released',
        description:
            '${run.checked} checks completed; ${run.held} stayed held.',
      ),
    };
    return SizedBox(
      height: 184,
      child: Center(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 40,
                height: 40,
                decoration: BoxDecoration(
                  color: context.colors.onSurface.withValues(alpha: 0.055),
                  shape: BoxShape.circle,
                ),
                child: Icon(
                  presentation.icon,
                  size: 17,
                  color: context.colors.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: Grid.sm),
              Text(
                presentation.title,
                style: context.textTheme.bodyMedium?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(height: Grid.xxs),
              Text(
                presentation.description,
                textAlign: TextAlign.center,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ReleaseTrackRow extends StatelessWidget {
  const _ReleaseTrackRow({required this.track});

  final ReleaseRunTrack track;

  @override
  Widget build(BuildContext context) {
    final destination = track.detailsUrl ?? track.sourceUrl;
    return Semantics(
      button: destination != null,
      label: 'Open ${track.artist} — ${track.title} in Trakd',
      child: InkWell(
        onTap: destination == null
            ? null
            : () async {
                Navigator.of(context).pop();
                await launchUrl(
                  Uri.parse(destination),
                  mode: LaunchMode.externalApplication,
                );
              },
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xs,
            vertical: Grid.twelve,
          ),
          child: Row(
            children: [
              _ReleaseArtwork(track: track),
              const SizedBox(width: Grid.twelve),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      track.artist,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: Grid.xxs),
                    Text.rich(
                      TextSpan(
                        text: track.title,
                        children: [
                          if (track.version != null)
                            TextSpan(
                              text: ' · ${track.version}',
                              style: TextStyle(
                                color: context.colors.onSurfaceVariant,
                                fontWeight: FontWeight.w400,
                              ),
                            ),
                        ],
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: context.colors.onSurface,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: Grid.xxs),
                    Text(
                      [
                        track.label,
                        track.releaseDate,
                        track.source,
                      ].whereType<String>().join(' · '),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: Grid.xxs),
              Icon(
                destination == null
                    ? LucideIcons.check
                    : LucideIcons.arrowUpRight,
                size: 17,
                color: destination == null
                    ? Colors.green
                    : context.colors.onSurfaceVariant,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ReleaseArtwork extends StatelessWidget {
  const _ReleaseArtwork({required this.track});

  final ReleaseRunTrack track;

  @override
  Widget build(BuildContext context) {
    final fallback = DecoratedBox(
      decoration: BoxDecoration(
        color: context.colors.onSurface.withValues(alpha: 0.055),
        borderRadius: BorderRadius.circular(Radii.md),
      ),
      child: Center(
        child: Icon(
          track.artworkUrl == null ? LucideIcons.music2 : LucideIcons.imageOff,
          size: 17,
          color: context.colors.onSurfaceVariant,
        ),
      ),
    );
    return SizedBox.square(
      dimension: 52,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(Radii.md),
        child: track.artworkUrl == null
            ? fallback
            : Image.network(
                track.artworkUrl!,
                fit: BoxFit.cover,
                errorBuilder: (_, _, _) => fallback,
              ),
      ),
    );
  }
}

class _ReleaseRunFooter extends StatelessWidget {
  const _ReleaseRunFooter({required this.sourceHealth});

  final String sourceHealth;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: context.colors.onSurface.withValues(alpha: 0.025),
        border: Border(
          top: BorderSide(
            color: context.colors.outlineVariant.withValues(alpha: 0.58),
          ),
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.xs,
          vertical: Grid.twelve,
        ),
        child: Row(
          children: [
            const Icon(LucideIcons.shieldCheck, size: 15, color: Colors.green),
            const SizedBox(width: Grid.xxs),
            Expanded(
              child: Text(
                sourceHealth,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
