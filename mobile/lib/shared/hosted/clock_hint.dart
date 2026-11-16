import 'dart:io' show HttpDate;

/// How far the local clock may differ from a server's `Date` before a refusal
/// gets a hint to check it.
const clockHintThreshold = Duration(seconds: 115);

/// The local clock's offset from a response's `Date` header in whole minutes,
/// when it is more than [clockHintThreshold] off; otherwise, or without a
/// readable `Date`, null. A hint never replaces the refusal it explains.
int? clockHintMinutes(String? date, DateTime localNow) {
  if (date == null) return null;
  final DateTime server;
  try {
    server = HttpDate.parse(date);
  } on Exception {
    return null;
  }
  final offset = localNow.difference(server).abs();
  if (offset <= clockHintThreshold) return null;
  return (offset.inSeconds / 60).round();
}

/// The line added under a refusal when [clockHintMinutes] found one.
String clockHintText(int minutes) =>
    "Your device's clock looks about $minutes minute${minutes == 1 ? '' : 's'} "
    'off, which can cause this. Check your date and time settings.';
