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

  /// Saves the state. Throws when the app group is unavailable or the avatar
  /// can't be written, leaving the previous Bestie in place.
  func save() throws {
    guard let group = Self.groupIdentifier, let defaults = UserDefaults(suiteName: group),
      let url = Self.avatarURL
    else { throw CocoaError(.fileNoSuchFile) }
    guard let name else {
      // Hiding the Bestie first means a leftover image is never shown.
      defaults.set(false, forKey: Self.presentKey)
      defaults.removeObject(forKey: Self.nameKey)
      try? FileManager.default.removeItem(at: url)
      return
    }
    // The image lands before the name, so a failed write never pairs one
    // Bestie's name with another's avatar.
    if let avatar {
      try avatar.write(to: url, options: .atomic)
    } else if FileManager.default.fileExists(atPath: url.path) {
      try FileManager.default.removeItem(at: url)
    }
    defaults.set(true, forKey: Self.presentKey)
    defaults.set(name, forKey: Self.nameKey)
  }
}
