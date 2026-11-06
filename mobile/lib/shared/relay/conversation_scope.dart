/// Read-only conversation metadata shared by feed consumers.
abstract interface class ConversationScope {
  String get id;
  String get name;
  String get channelType;
  bool get isMember;
  bool get isArchived;
  bool get isDm;
  bool get isSolo;
}
