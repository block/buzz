part of '../channel_detail_page.dart';

const _huddleClusterDesignSize = Size(360, 340);
const _huddleClusterVisibleParticipantCount = 10;
const _huddleClusterFrameScale =
    _huddleAvatarFrameSize / (_huddleAvatarRadius * 2);
const _huddleParticipantPresenceScale = 0.72;

const _huddleClusterSlots = <_HuddleClusterSlot>[
  // A: lightly emphasized center participant.
  _HuddleClusterSlot(left: 130, top: 120, avatarDiameter: 100),
  // B-F: the original lower/right sequence.
  _HuddleClusterSlot(left: 226.2, top: 203.3, avatarDiameter: 88),
  _HuddleClusterSlot(left: 157.2, top: 228.5, avatarDiameter: 64),
  _HuddleClusterSlot(left: 255.3, top: 135.2, avatarDiameter: 64),
  _HuddleClusterSlot(left: 223.8, top: 108.8, avatarDiameter: 40),
  _HuddleClusterSlot(left: 103.2, top: 236, avatarDiameter: 48),
  // G-J: the mirrored upper/left sequence.
  _HuddleClusterSlot(left: 45.8, top: 48.7, avatarDiameter: 88),
  _HuddleClusterSlot(left: 138.8, top: 47.5, avatarDiameter: 64),
  _HuddleClusterSlot(left: 40.7, top: 140.8, avatarDiameter: 64),
  _HuddleClusterSlot(left: 96.2, top: 191.2, avatarDiameter: 40),
];

const _huddleClusterOverflowSlot = _HuddleClusterSlot(
  left: 208.8,
  top: 56,
  avatarDiameter: 48,
);

final _huddleClusterAnchor = _huddleClusterCentroid([
  ..._huddleClusterSlots,
  _huddleClusterOverflowSlot,
]);

class _HuddleParticipantCluster extends HookWidget {
  const _HuddleParticipantCluster({
    required this.pubkeys,
    required this.profiles,
    required this.fallbackLabels,
    required this.activeSpeakerPubkeys,
    required this.speakerLevels,
    required this.movementDuration,
    required this.entryDuration,
    required this.exitDuration,
    required this.onParticipantTap,
    required this.onOverflowTap,
  });

  final List<String> pubkeys;
  final Map<String, UserProfile> profiles;
  final Map<String, String> fallbackLabels;
  final Set<String> activeSpeakerPubkeys;
  final Map<String, double> speakerLevels;
  final Duration movementDuration;
  final Duration entryDuration;
  final Duration exitDuration;
  final ValueChanged<String> onParticipantTap;
  final VoidCallback onOverflowTap;

  @override
  Widget build(BuildContext context) {
    final visiblePubkeys = pubkeys
        .take(_huddleClusterVisibleParticipantCount)
        .toList(growable: false);
    final visiblePubkeySet = visiblePubkeys.toSet();
    final renderedPubkeys = useState<List<String>>(visiblePubkeys);
    final removalTimers = useMemoized(() => <String, Timer>{});
    final lastSlots = useMemoized(() => <String, _HuddleClusterSlot>{});
    useEffect(() {
      for (final pubkey in visiblePubkeys) {
        removalTimers.remove(pubkey)?.cancel();
      }

      final nextRenderedPubkeys = [...renderedPubkeys.value];
      for (final pubkey in visiblePubkeys) {
        if (!nextRenderedPubkeys.contains(pubkey)) {
          nextRenderedPubkeys.add(pubkey);
        }
      }
      if (!listEquals(renderedPubkeys.value, nextRenderedPubkeys)) {
        renderedPubkeys.value = List.unmodifiable(nextRenderedPubkeys);
      }

      for (final pubkey in renderedPubkeys.value) {
        if (visiblePubkeySet.contains(pubkey) ||
            removalTimers.containsKey(pubkey)) {
          continue;
        }
        removalTimers[pubkey] = Timer(exitDuration, () {
          removalTimers.remove(pubkey);
          lastSlots.remove(pubkey);
          renderedPubkeys.value = List.unmodifiable(
            renderedPubkeys.value.where((value) => value != pubkey),
          );
        });
      }
      return null;
    }, [Object.hashAll(visiblePubkeys), exitDuration]);
    useEffect(
      () => () {
        for (final timer in removalTimers.values) {
          timer.cancel();
        }
        removalTimers.clear();
      },
      const [],
    );
    final overflowCount =
        pubkeys.length - _huddleClusterVisibleParticipantCount;
    final layout = _huddleClusterLayout(
      participantCount: visiblePubkeys.length,
      showsOverflow: overflowCount > 0,
    );
    for (final (index, pubkey) in visiblePubkeys.indexed) {
      lastSlots[pubkey] = layout.participantSlots[index];
    }
    final orderedRenderedPubkeys = [
      for (final pubkey in renderedPubkeys.value)
        if (!visiblePubkeySet.contains(pubkey)) pubkey,
      for (final pubkey in visiblePubkeys)
        if (renderedPubkeys.value.contains(pubkey)) pubkey,
    ];

    return FittedBox(
      key: const ValueKey('huddle-participant-cluster'),
      fit: BoxFit.contain,
      child: SizedBox(
        width: _huddleClusterDesignSize.width,
        height: _huddleClusterDesignSize.height,
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            for (final pubkey in orderedRenderedPubkeys)
              _HuddleAnimatedParticipant(
                key: ValueKey('huddle-participant-entry-$pubkey'),
                pubkey: pubkey,
                profile: profiles[pubkey],
                fallbackLabel: fallbackLabels[pubkey],
                active: activeSpeakerPubkeys.contains(pubkey),
                speakerLevel: speakerLevels[pubkey] ?? 0,
                slot: lastSlots[pubkey]!,
                present: visiblePubkeySet.contains(pubkey),
                movementDuration: movementDuration,
                entryDuration: entryDuration,
                exitDuration: exitDuration,
                onTap: visiblePubkeySet.contains(pubkey)
                    ? () {
                        unawaited(HapticFeedback.selectionClick());
                        onParticipantTap(pubkey);
                      }
                    : null,
              ),
            if (overflowCount > 0)
              _HuddleParticipantOverflow(
                count: overflowCount,
                slot: layout.overflowSlot!,
                entryDuration: entryDuration,
                onTap: onOverflowTap,
              ),
          ],
        ),
      ),
    );
  }
}

class _HuddleAnimatedParticipant extends StatelessWidget {
  const _HuddleAnimatedParticipant({
    super.key,
    required this.pubkey,
    required this.profile,
    required this.fallbackLabel,
    required this.active,
    required this.speakerLevel,
    required this.slot,
    required this.present,
    required this.movementDuration,
    required this.entryDuration,
    required this.exitDuration,
    required this.onTap,
  });

  final String pubkey;
  final UserProfile? profile;
  final String? fallbackLabel;
  final bool active;
  final double speakerLevel;
  final _HuddleClusterSlot slot;
  final bool present;
  final Duration movementDuration;
  final Duration entryDuration;
  final Duration exitDuration;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final frameSize = slot.avatarDiameter * _huddleClusterFrameScale;
    final frameInset = (frameSize - slot.avatarDiameter) / 2;
    final targetOffset = Offset(slot.left - frameInset, slot.top - frameInset);

    return Positioned(
      left: 0,
      top: 0,
      width: frameSize,
      height: frameSize + _huddleParticipantLabelSpace,
      child: TweenAnimationBuilder<Offset>(
        key: ValueKey('huddle-participant-motion-$pubkey'),
        duration: movementDuration,
        curve: Curves.easeOutBack,
        tween: Tween(begin: targetOffset, end: targetOffset),
        builder: (context, offset, child) => Transform.translate(
          key: ValueKey('huddle-participant-translation-$pubkey'),
          offset: offset,
          child: child,
        ),
        child: TweenAnimationBuilder<double>(
          key: ValueKey('huddle-participant-entry-motion-$pubkey'),
          duration: present ? entryDuration : exitDuration,
          curve: present
              ? Curves.easeOutBack
              : const _HuddleExitPresenceCurve(),
          tween: Tween(begin: present ? 0 : 1, end: present ? 1 : 0),
          builder: (context, value, child) => Opacity(
            opacity: value.clamp(0, 1),
            child: Transform.scale(
              key: ValueKey('huddle-participant-entry-scale-$pubkey'),
              scale:
                  _huddleParticipantPresenceScale +
                  value * (1 - _huddleParticipantPresenceScale),
              child: child,
            ),
          ),
          child: _HuddleCallAvatar(
            pubkey: pubkey,
            profile: profile,
            fallbackLabel: fallbackLabel,
            active: active,
            speakerLevel: speakerLevel,
            frameSize: frameSize,
            onTap: onTap,
          ),
        ),
      ),
    );
  }
}

class _HuddleParticipantOverflow extends StatelessWidget {
  const _HuddleParticipantOverflow({
    required this.count,
    required this.slot,
    required this.entryDuration,
    required this.onTap,
  });

  final int count;
  final _HuddleClusterSlot slot;
  final Duration entryDuration;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: slot.left,
      top: slot.top,
      width: slot.avatarDiameter,
      height: slot.avatarDiameter,
      child: TweenAnimationBuilder<double>(
        duration: entryDuration,
        curve: Curves.easeOutBack,
        tween: Tween(begin: 0, end: 1),
        builder: (context, value, child) => Opacity(
          opacity: value.clamp(0, 1),
          child: Transform.scale(scale: 0.95 + value * 0.05, child: child),
        ),
        child: Semantics(
          label: '$count more participants',
          hint: 'Tap to show participant list',
          button: true,
          onTap: onTap,
          child: ExcludeSemantics(
            child: GestureDetector(
              key: const ValueKey('huddle-participant-overflow'),
              behavior: HitTestBehavior.opaque,
              excludeFromSemantics: true,
              onTap: onTap,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: context.colors.surfaceContainerHighest,
                ),
                child: Center(
                  child: AnimatedSwitcher(
                    duration: entryDuration,
                    switchInCurve: Curves.easeOutCubic,
                    switchOutCurve: Curves.easeInCubic,
                    child: Text(
                      '+$count',
                      key: ValueKey('huddle-participant-overflow-count-$count'),
                      style: context.textTheme.titleMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _HuddleClusterSlot {
  const _HuddleClusterSlot({
    required this.left,
    required this.top,
    required this.avatarDiameter,
  });

  final double left;
  final double top;
  final double avatarDiameter;

  _HuddleClusterSlot translate(Offset offset) => _HuddleClusterSlot(
    left: left + offset.dx,
    top: top + offset.dy,
    avatarDiameter: avatarDiameter,
  );
}

_HuddleClusterLayout _huddleClusterLayout({
  required int participantCount,
  required bool showsOverflow,
}) {
  final participantSlots = _huddleClusterSlots
      .take(participantCount)
      .toList(growable: false);
  final occupiedSlots = [
    ...participantSlots,
    if (showsOverflow) _huddleClusterOverflowSlot,
  ];
  if (occupiedSlots.isEmpty) {
    return const _HuddleClusterLayout(participantSlots: []);
  }

  final translation =
      _huddleClusterAnchor - _huddleClusterCentroid(occupiedSlots);
  return _HuddleClusterLayout(
    participantSlots: [
      for (final slot in participantSlots) slot.translate(translation),
    ],
    overflowSlot: showsOverflow
        ? _huddleClusterOverflowSlot.translate(translation)
        : null,
  );
}

Offset _huddleClusterCentroid(Iterable<_HuddleClusterSlot> slots) {
  var x = 0.0;
  var y = 0.0;
  var count = 0;
  for (final slot in slots) {
    x += slot.left + slot.avatarDiameter / 2;
    y += slot.top + slot.avatarDiameter / 2;
    count++;
  }
  return count == 0 ? Offset.zero : Offset(x / count, y / count);
}

class _HuddleClusterLayout {
  const _HuddleClusterLayout({
    required this.participantSlots,
    this.overflowSlot,
  });

  final List<_HuddleClusterSlot> participantSlots;
  final _HuddleClusterSlot? overflowSlot;
}

class _HuddleExitPresenceCurve extends Curve {
  const _HuddleExitPresenceCurve();

  @override
  double transformInternal(double t) {
    const overshootEnd = 0.3;
    const overshootPeak = overshootEnd / 2;
    const overshootAmount = 0.25;
    if (t < overshootPeak) {
      return -overshootAmount *
          Curves.easeOutCubic.transform(t / overshootPeak);
    }
    if (t < overshootEnd) {
      return -overshootAmount *
          (1 -
              Curves.easeInOutCubic.transform(
                (t - overshootPeak) / overshootPeak,
              ));
    }
    return Curves.easeInOutCubic.transform(
      (t - overshootEnd) / (1 - overshootEnd),
    );
  }
}
