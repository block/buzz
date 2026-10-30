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
