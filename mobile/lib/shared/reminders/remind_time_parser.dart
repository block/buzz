/// Parser for `/remind me <what> <when>` — the grammar in
/// `docs/slash-commands.md`. Desktop implements the same grammar; both apps
/// must produce the same note and due time for every vector in that spec.
library;

/// Converts between instants and wall-clock time in one time zone.
///
/// Days, weeks and weekdays are wall-clock arithmetic, so they keep the same
/// local time across a DST change. The production zone is the device's;
/// tests inject a fixed zone so the DST vectors do not depend on the process
/// `TZ`.
abstract interface class RemindZone {
  /// The instant at which the zone's clock reads the given wall time. Fields
  /// outside their normal range roll over, as in the [DateTime] constructor.
  DateTime wallToInstant(int year, int month, int day, int hour, int minute);

  /// [instant] with its fields reading the zone's wall-clock time.
  DateTime instantToWall(DateTime instant);
}

/// The device's local time zone.
class LocalRemindZone implements RemindZone {
  const LocalRemindZone();

  @override
  DateTime wallToInstant(int year, int month, int day, int hour, int minute) =>
      DateTime(year, month, day, hour, minute);

  @override
  DateTime instantToWall(DateTime instant) => instant.toLocal();
}

/// Why a `/remind` command line was rejected.
enum RemindParseError {
  /// Nothing after `/remind`, or nothing after `me`.
  usage(null),

  /// The target is not `me`.
  notPersonal('Reminders are personal. Use /remind me …'),

  /// A time was given but no note.
  missingNote('Say what to be reminded about.'),

  /// A note was given but no recognizable time.
  missingTime(
    'Say when, such as “in 20 minutes”, “at 3pm” or “tomorrow at 4pm”.',
  ),

  /// An hour with neither am/pm nor a colon.
  ambiguousHour('Add am or pm to the hour, such as “at 3pm”.'),

  /// A time that does not exist, such as `at 25:00` or `in 0 minutes`.
  invalidTime('That is not a valid time.'),

  /// A due time that is not after now, or is more than 365 days ahead.
  outOfRange('Pick a time within the next 365 days.');

  /// The text to show above the usage line; null shows the usage line alone.
  final String? message;

  const RemindParseError(this.message);
}

/// The outcome of [parseRemindCommand].
sealed class RemindParseResult {
  const RemindParseResult();
}

/// A valid command: remind the user of [note] at [dueAt].
class RemindParsed extends RemindParseResult {
  final String note;
  final DateTime dueAt;

  const RemindParsed(this.note, this.dueAt);
}

/// An invalid command.
class RemindRejected extends RemindParseResult {
  final RemindParseError error;

  const RemindRejected(this.error);
}

const _maxLeadDays = 365;
const _defaultHour = 9;

const _timeSource = r'(?:noon|midnight|\d{1,2}(?::\d{2})?(?:\s*(?:am|pm))?)';
const _weekdayNames = [
  'monday',
  'tuesday',
  'wednesday',
  'thursday',
  'friday',
  'saturday',
  'sunday',
];

final List<RegExp> _whenForms = [
  RegExp(
    r'in\s+(?:\d+|an?)\s+(?:minutes?|mins?|hours?|hrs?|days?|weeks?)',
    caseSensitive: false,
  ),
  RegExp('at\\s+$_timeSource', caseSensitive: false),
  RegExp('tomorrow(?:\\s+at\\s+$_timeSource)?', caseSensitive: false),
  RegExp(
    '(?:(?:on|next)\\s+)?(?:${_weekdayNames.join('|')})'
    '(?:\\s+at\\s+$_timeSource)?',
    caseSensitive: false,
  ),
  RegExp(r'next\s+week', caseSensitive: false),
];

final _meTarget = RegExp(r'^me(?:\s+|$)', caseSensitive: false);
final _leadingTo = RegExp(r'^to(?:\s+|$)', caseSensitive: false);
final _inPattern = RegExp(
  r'^in\s+(\d+|an?)\s+(minute|min|hour|hr|day|week)s?$',
  caseSensitive: false,
);
final _timePattern = RegExp(
  r'^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$',
  caseSensitive: false,
);
final _atTailPattern = RegExp(
  '\\s+at\\s+($_timeSource)\$',
  caseSensitive: false,
);

/// Parses the text after `/remind`. [now] is injected; [zone] defaults to the
/// device's.
RemindParseResult parseRemindCommand(
  String args, {
  required DateTime now,
  RemindZone zone = const LocalRemindZone(),
}) {
  final text = args.trim();
  if (text.isEmpty) return const RemindRejected(RemindParseError.usage);
  final target = _meTarget.firstMatch(text);
  if (target == null) {
    return const RemindRejected(RemindParseError.notPersonal);
  }
  final rest = text.substring(target.end).trim();
  if (rest.isEmpty) return const RemindRejected(RemindParseError.usage);

  final (phrase, noteSource) = _splitWhen(rest);
  if (phrase == null) {
    return const RemindRejected(RemindParseError.missingTime);
  }
  final note = noteSource.trim().replaceFirst(_leadingTo, '').trim();

  final DateTime dueAt;
  try {
    dueAt = _resolve(phrase, now, zone);
  } on _Rejected catch (rejected) {
    return RemindRejected(rejected.error);
  }
  if (note.isEmpty) return const RemindRejected(RemindParseError.missingNote);
  if (!dueAt.isAfter(now) ||
      dueAt.difference(now) > const Duration(days: _maxLeadDays)) {
    return const RemindRejected(RemindParseError.outOfRange);
  }
  return RemindParsed(note, dueAt);
}

/// Finds `<when>` at the end of [text], else at the start; at each end the
/// longest phrase wins. Returns the phrase and the text outside it.
(String?, String) _splitWhen(String text) {
  Match? best;
  for (final form in _whenForms) {
    final match = RegExp(
      '(?<!\\S)(?:${form.pattern})\$',
      caseSensitive: false,
    ).firstMatch(text);
    if (match != null && (best == null || _length(match) > _length(best))) {
      best = match;
    }
  }
  if (best != null) {
    return (best.group(0), text.substring(0, best.start));
  }

  for (final form in _whenForms) {
    final match = RegExp(
      '^(?:${form.pattern})(?=\\s|\$)',
      caseSensitive: false,
    ).firstMatch(text);
    if (match != null && (best == null || _length(match) > _length(best))) {
      best = match;
    }
  }
  if (best != null) {
    return (best.group(0), text.substring(best.end));
  }
  return (null, text);
}

int _length(Match match) => match.end - match.start;

class _Rejected implements Exception {
  final RemindParseError error;

  const _Rejected(this.error);
}

/// The due instant for a matched `<when>` phrase; throws [_Rejected] for a
/// phrase that matches the grammar but names no valid time.
DateTime _resolve(String when, DateTime now, RemindZone zone) {
  final phrase = when.trim().toLowerCase();
  final wall = zone.instantToWall(now);

  DateTime atWall(int dayOffset, int hour, int minute) => zone.wallToInstant(
    wall.year,
    wall.month,
    wall.day + dayOffset,
    hour,
    minute,
  );

  final relative = _inPattern.firstMatch(phrase);
  if (relative != null) {
    final amount = switch (relative.group(1)!) {
      'a' || 'an' => 1,
      final digits => int.tryParse(digits) ?? 0,
    };
    if (amount < 1) throw const _Rejected(RemindParseError.invalidTime);
    switch (relative.group(2)!) {
      case 'minute' || 'min':
        return now.add(Duration(minutes: amount));
      case 'hour' || 'hr':
        return now.add(Duration(hours: amount));
      case 'day':
        return atWall(amount, wall.hour, wall.minute);
      default:
        return atWall(amount * 7, wall.hour, wall.minute);
    }
  }

  if (phrase == 'next week') {
    return atWall(_daysAfterToday(wall, DateTime.monday), _defaultHour, 0);
  }

  final timeTail = _atTailPattern.firstMatch(phrase);
  final String head;
  ({int hour, int minute})? clock;
  if (timeTail != null) {
    clock = _parseClock(timeTail.group(1)!);
    head = phrase.substring(0, timeTail.start);
  } else if (phrase.startsWith('at ')) {
    clock = _parseClock(phrase.substring(3).trim());
    head = '';
  } else {
    head = phrase;
  }

  if (head.isEmpty) {
    final today = atWall(0, clock!.hour, clock.minute);
    return today.isAfter(now) ? today : atWall(1, clock.hour, clock.minute);
  }
  final hour = clock?.hour ?? _defaultHour;
  final minute = clock?.minute ?? 0;
  if (head == 'tomorrow') return atWall(1, hour, minute);

  final dayName = head.split(RegExp(r'\s+')).last;
  final weekday = _weekdayNames.indexOf(dayName);
  if (weekday < 0) throw const _Rejected(RemindParseError.missingTime);
  return atWall(_daysAfterToday(wall, weekday + 1), hour, minute);
}

int _daysAfterToday(DateTime wall, int weekday) {
  final days = (weekday - wall.weekday) % 7;
  return days == 0 ? 7 : days;
}

({int hour, int minute}) _parseClock(String source) {
  switch (source) {
    case 'noon':
      return (hour: 12, minute: 0);
    case 'midnight':
      return (hour: 0, minute: 0);
  }
  final match = _timePattern.firstMatch(source);
  if (match == null) throw const _Rejected(RemindParseError.invalidTime);
  final hour = int.parse(match.group(1)!);
  final minuteText = match.group(2);
  final minute = minuteText == null ? 0 : int.parse(minuteText);
  final meridiem = match.group(3);
  if (minute > 59) throw const _Rejected(RemindParseError.invalidTime);
  if (meridiem == null) {
    if (minuteText == null) {
      throw const _Rejected(RemindParseError.ambiguousHour);
    }
    if (hour > 23) throw const _Rejected(RemindParseError.invalidTime);
    return (hour: hour, minute: minute);
  }
  if (hour < 1 || hour > 12) {
    throw const _Rejected(RemindParseError.invalidTime);
  }
  return (hour: hour % 12 + (meridiem == 'pm' ? 12 : 0), minute: minute);
}
