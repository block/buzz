/// Builds the kind:9002 tags used to update channel name and description.
List<List<String>> buildUpdateChannelTags({
  required String channelId,
  String? name,
  String? description,
}) {
  if (name == null && description == null) {
    throw ArgumentError('At least one channel field must be provided');
  }
  final canonicalName = name?.trim().replaceFirst(RegExp(r'^#+'), '').trim();
  if (canonicalName != null && canonicalName.isEmpty) {
    throw ArgumentError.value(name, 'name', 'must not be empty');
  }
  return [
    ['h', channelId],
    if (canonicalName != null) ['name', canonicalName],
    if (description != null) ['about', description.trim()],
  ];
}

/// Convert the existing stream without changing its visibility or join role.
List<List<String>> buildSoloConversionTags(
  String channelId,
  List<String> labels,
) {
  final retained = labels.where((label) => label != 'buzz.solo');
  return [
    ['h', channelId],
    ['unlisted', 'false'],
    if (retained.isEmpty) ['t', ''],
    for (final label in retained) ['t', label],
  ];
}
