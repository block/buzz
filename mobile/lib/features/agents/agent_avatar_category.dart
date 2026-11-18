import '../../shared/agents/agent_avatar_catalog.dart';

/// One destination in the avatar category rail.
class AgentAvatarCategory {
  const AgentAvatarCategory.pack(AgentAvatarPack value)
    : pack = value,
      _extra = 0;
  const AgentAvatarCategory._(this._extra) : pack = null;
  static const emoji = AgentAvatarCategory._(0);
  static const image = AgentAvatarCategory._(1);
  final AgentAvatarPack? pack;
  final int _extra;
  int get position => pack?.index ?? AgentAvatarPack.values.length + _extra;
  @override
  bool operator ==(Object other) =>
      other is AgentAvatarCategory &&
      pack == other.pack &&
      _extra == other._extra;
  @override
  int get hashCode => Object.hash(pack, _extra);
}
