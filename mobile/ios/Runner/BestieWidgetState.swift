import Foundation

/// The Bestie shown by the lock screen widget, shared through the app group.
///
/// The app writes it whenever Home learns the current community's Bestie; the
/// widget extension only reads it.
struct BestieWidgetState {
  static let kind = "BestieWidget"
  private static let nameKey = "bestieWidget.name"
  private static let presentKey = "bestieWidget.present"
  private static let avatarFile = "BestieWidgetAvatar.png"

  /// The Bestie's display name, or nil when the community has no Bestie.
  var name: String?
  /// PNG artwork for the Bestie's avatar, when one could be rendered.
  var avatar: Data?

  private static var groupIdentifier: String? {
    Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String
  }

  private static var avatarURL: URL? {
    guard let group = groupIdentifier else { return nil }
    return FileManager.default
      .containerURL(forSecurityApplicationGroupIdentifier: group)?
      .appendingPathComponent(avatarFile)
  }

  static func load() -> BestieWidgetState {
    guard let group = groupIdentifier, let defaults = UserDefaults(suiteName: group),
      defaults.bool(forKey: presentKey)
    else { return BestieWidgetState() }
    return BestieWidgetState(
      name: defaults.string(forKey: nameKey),
      avatar: avatarURL.flatMap { try? Data(contentsOf: $0) })
  }

  /// Saves the state, returning false when the app group is unavailable.
  @discardableResult
  func save() -> Bool {
    guard let group = Self.groupIdentifier, let defaults = UserDefaults(suiteName: group)
    else { return false }
    if let url = Self.avatarURL {
      if let avatar { try? avatar.write(to: url, options: .atomic) } else {
        try? FileManager.default.removeItem(at: url)
      }
    }
    defaults.set(name != nil, forKey: Self.presentKey)
    defaults.set(name, forKey: Self.nameKey)
    return true
  }
}
