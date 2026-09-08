const _millisecondsPerSecond = 1000;

int? dateTimeToUnixSeconds(DateTime? value) {
  if (value == null) return null;
  return value.millisecondsSinceEpoch ~/ _millisecondsPerSecond;
}

int currentUnixSeconds() => dateTimeToUnixSeconds(DateTime.now())!;

DateTime unixSecondsToDateTime(int value) {
  return DateTime.fromMillisecondsSinceEpoch(
    value * _millisecondsPerSecond,
    isUtc: true,
  );
}

int? isoToUnixSeconds(Object? value) {
  if (value is! String || value.isEmpty) {
    return null;
  }

  return dateTimeToUnixSeconds(DateTime.tryParse(value));
}

const readStateMaxClockDriftSeconds = 300;
