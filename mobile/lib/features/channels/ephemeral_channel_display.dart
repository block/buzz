import 'dart:math' as math;

import 'channel.dart';

class EphemeralChannelDisplay {
  final String? detailLabel;
  final String tooltipLabel;

  const EphemeralChannelDisplay({
    required this.detailLabel,
    required this.tooltipLabel,
  });
}

bool isEphemeralChannel(Channel channel) =>
    channel.ttlSeconds != null || channel.ttlDeadline != null;

EphemeralChannelDisplay? ephemeralChannelDisplay(
  Channel channel, {
  DateTime? now,
}) {
  if (!isEphemeralChannel(channel)) return null;

  final deadline = channel.ttlDeadline;
  final remainingSeconds = deadline == null
      ? null
      : ((deadline.millisecondsSinceEpoch -
                    (now ?? DateTime.now()).millisecondsSinceEpoch) /
                1000)
            .ceil();

  if (remainingSeconds == null) {
    final ttlSeconds = channel.ttlSeconds;
    return EphemeralChannelDisplay(
      detailLabel: ttlSeconds == null ? null : formatCompactTtl(ttlSeconds),
      tooltipLabel: ttlSeconds == null
          ? 'Ephemeral channel. Cleans up automatically after inactivity.'
          : 'Ephemeral channel. Cleans up after ${formatVerboseTtl(ttlSeconds)} of inactivity.',
    );
  }

  final compactRemaining = formatCompactRemaining(remainingSeconds);
  final verboseRemaining = formatVerboseRemaining(remainingSeconds);
  final absoluteDeadlineLabel = formatAbsoluteDeadline(deadline!);

  return EphemeralChannelDisplay(
    detailLabel: compactRemaining,
    tooltipLabel: compactRemaining == 'Cleanup due'
        ? 'Ephemeral channel. Cleanup is due now.'
        : 'Ephemeral channel. Cleans up $verboseRemaining. Scheduled for $absoluteDeadlineLabel.',
  );
}

String formatCompactRemaining(int remainingSeconds) {
  if (remainingSeconds <= 0) return 'Cleanup due';
  if (remainingSeconds <= 60) return '1m left';
  if (remainingSeconds < 60 * 60) {
    return '${math.max(1, (remainingSeconds / 60).ceil())}m left';
  }
  if (remainingSeconds < 60 * 60 * 24) {
    return '${math.max(1, (remainingSeconds / (60 * 60)).ceil())}h left';
  }
  return '${math.max(1, (remainingSeconds / (60 * 60 * 24)).ceil())}d left';
}

String formatVerboseRemaining(int remainingSeconds) {
  if (remainingSeconds <= 0) return 'now';
  if (remainingSeconds <= 60) return 'in 1 minute';
  if (remainingSeconds < 60 * 60) {
    final minutes = math.max(1, (remainingSeconds / 60).ceil());
    return 'in $minutes minute${minutes == 1 ? '' : 's'}';
  }
  if (remainingSeconds < 60 * 60 * 24) {
    final hours = math.max(1, (remainingSeconds / (60 * 60)).ceil());
    return 'in $hours hour${hours == 1 ? '' : 's'}';
  }
  final days = math.max(1, (remainingSeconds / (60 * 60 * 24)).ceil());
  return 'in $days day${days == 1 ? '' : 's'}';
}

String formatCompactTtl(int ttlSeconds) {
  if (ttlSeconds < 60) return '${math.max(1, ttlSeconds)}s TTL';
  if (ttlSeconds < 60 * 60) {
    return '${math.max(1, (ttlSeconds / 60).ceil())}m TTL';
  }
  if (ttlSeconds < 60 * 60 * 24) {
    return '${math.max(1, (ttlSeconds / (60 * 60)).ceil())}h TTL';
  }
  return '${math.max(1, (ttlSeconds / (60 * 60 * 24)).ceil())}d TTL';
}

String formatVerboseTtl(int ttlSeconds) {
  if (ttlSeconds < 60) {
    final seconds = math.max(1, ttlSeconds);
    return '$seconds second${seconds == 1 ? '' : 's'}';
  }
  if (ttlSeconds < 60 * 60) {
    final minutes = math.max(1, (ttlSeconds / 60).ceil());
    return '$minutes minute${minutes == 1 ? '' : 's'}';
  }
  if (ttlSeconds < 60 * 60 * 24) {
    final hours = math.max(1, (ttlSeconds / (60 * 60)).ceil());
    return '$hours hour${hours == 1 ? '' : 's'}';
  }
  final days = math.max(1, (ttlSeconds / (60 * 60 * 24)).ceil());
  return '$days day${days == 1 ? '' : 's'}';
}

String formatAbsoluteDeadline(DateTime deadline) {
  const months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];
  final local = deadline.toLocal();
  final hour12 = local.hour % 12 == 0 ? 12 : local.hour % 12;
  final minute = local.minute.toString().padLeft(2, '0');
  final period = local.hour < 12 ? 'AM' : 'PM';
  return '${months[local.month - 1]} ${local.day}, $hour12:$minute $period';
}
