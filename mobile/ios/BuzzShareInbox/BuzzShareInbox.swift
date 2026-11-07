import Foundation

/// App Group hand-off between the Share Extension and the main app.
///
/// The extension copies every shared item into
/// `<app group>/Library/Caches/share-inbox/<id>/` and writes a `manifest.json`
/// next to the copies. The app takes the newest manifest, hands the files to
/// the composer, and discards the manifest once it has been consumed. This file
/// is compiled into both targets; keep it dependency-free.
enum BuzzShareInbox {
  static let directoryName = "share-inbox"
  static let manifestName = "manifest.json"
  static let maximumItems = 10
  /// Payloads older than this are pruned on the next read.
  static let staleInterval: TimeInterval = 24 * 60 * 60

  struct Item: Codable, Equatable {
    enum Kind: String, Codable {
      case text
      case url
      case file
    }

    let kind: Kind
    /// Text body or URL string. Nil for files.
    let value: String?
    /// Absolute path of the copied file inside the payload directory.
    let path: String?
    let name: String?
    let mimeType: String?

    var flutterArguments: [String: Any] {
      var map: [String: Any] = ["kind": kind.rawValue]
      if let value { map["value"] = value }
      if let path { map["path"] = path }
      if let name { map["name"] = name }
      if let mimeType { map["mimeType"] = mimeType }
      return map
    }
  }

  /// Destination the user already picked inside the extension, if any.
  struct Target: Codable, Equatable {
    let communityID: String
    let channelID: String
  }

  struct Payload: Codable, Equatable {
    let id: String
    let createdAt: TimeInterval
    let items: [Item]
    var target: Target?

    init(id: String, createdAt: TimeInterval, items: [Item], target: Target? = nil) {
      self.id = id
      self.createdAt = createdAt
      self.items = items
      self.target = target
    }

    var flutterArguments: [String: Any] {
      var map: [String: Any] = [
        "id": id,
        "createdAt": createdAt,
        "items": items.map(\.flutterArguments),
      ]
      if let target {
        map["communityId"] = target.communityID
        map["channelId"] = target.channelID
      }
      return map
    }
  }

  static func containerURL(appGroupIdentifier: String?) -> URL? {
    guard let appGroupIdentifier, !appGroupIdentifier.isEmpty else { return nil }
    return FileManager.default.containerURL(
      forSecurityApplicationGroupIdentifier: appGroupIdentifier
    )
  }

  static func inboxURL(appGroupIdentifier: String?) -> URL? {
    guard let container = containerURL(appGroupIdentifier: appGroupIdentifier) else {
      return nil
    }
    return
      container
      .appendingPathComponent("Library", isDirectory: true)
      .appendingPathComponent("Caches", isDirectory: true)
      .appendingPathComponent(directoryName, isDirectory: true)
  }

  /// Creates a fresh, empty payload directory and returns it with its id.
  static func makePayloadDirectory(appGroupIdentifier: String?) throws -> (id: String, url: URL) {
    guard let inbox = inboxURL(appGroupIdentifier: appGroupIdentifier) else {
      throw CocoaError(.fileNoSuchFile)
    }
    let id = UUID().uuidString.lowercased()
    let directory = inbox.appendingPathComponent(id, isDirectory: true)
    try FileManager.default.createDirectory(
      at: directory,
      withIntermediateDirectories: true
    )
    return (id, directory)
  }

  static func writeManifest(_ payload: Payload, in directory: URL) throws {
    let data = try JSONEncoder().encode(payload)
    try data.write(to: directory.appendingPathComponent(manifestName), options: .atomic)
  }

  /// Returns the newest pending payload, pruning stale directories first.
  static func takePending(appGroupIdentifier: String?) -> Payload? {
    guard let inbox = inboxURL(appGroupIdentifier: appGroupIdentifier) else { return nil }
    let fileManager = FileManager.default
    guard
      let directories = try? fileManager.contentsOfDirectory(
        at: inbox,
        includingPropertiesForKeys: nil
      )
    else { return nil }
    let now = Date().timeIntervalSince1970
    var newest: Payload?
    for directory in directories {
      let manifest = directory.appendingPathComponent(manifestName)
      guard let data = try? Data(contentsOf: manifest),
        let payload = try? JSONDecoder().decode(Payload.self, from: data)
      else {
        // A directory without a readable manifest was either consumed or
        // abandoned mid-write; drop it once it is old enough.
        prune(directory, olderThan: now - staleInterval)
        continue
      }
      if now - payload.createdAt > staleInterval {
        try? fileManager.removeItem(at: directory)
        continue
      }
      // The manifest must describe its own directory and may only reference
      // files inside it; anything else is dropped rather than trusted.
      guard payload.id == directory.lastPathComponent,
        payload.items.allSatisfy({ isContained($0.path, in: directory) })
      else {
        try? fileManager.removeItem(at: directory)
        continue
      }
      if let current = newest, current.createdAt >= payload.createdAt { continue }
      newest = payload
    }
    return newest
  }

  /// Deletes staged payloads older than [staleInterval], including directories
  /// whose manifest was never written (a share dismissed mid-way). Called by
  /// the extension too, so the inbox stays bounded even if the app is never
  /// opened.
  static func pruneStale(appGroupIdentifier: String?) {
    guard let inbox = inboxURL(appGroupIdentifier: appGroupIdentifier),
      let directories = try? FileManager.default.contentsOfDirectory(
        at: inbox,
        includingPropertiesForKeys: nil
      )
    else { return }
    let cutoff = Date().timeIntervalSince1970 - staleInterval
    for directory in directories {
      prune(directory, olderThan: cutoff)
    }
  }

  private static func isContained(_ path: String?, in directory: URL) -> Bool {
    guard let path else { return true }
    let file = URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath().path
    let root = directory.standardizedFileURL.resolvingSymlinksInPath().path
    return file.hasPrefix(root + "/")
  }

  /// Removes the manifest so the payload is not surfaced again. Copied files
  /// stay in place until the composer deletes them after upload.
  static func discard(id: String, appGroupIdentifier: String?) {
    guard let inbox = inboxURL(appGroupIdentifier: appGroupIdentifier),
      isSafeIdentifier(id)
    else { return }
    let directory = inbox.appendingPathComponent(id, isDirectory: true)
    try? FileManager.default.removeItem(at: directory.appendingPathComponent(manifestName))
  }

  private static func prune(_ directory: URL, olderThan cutoff: TimeInterval) {
    let attributes = try? FileManager.default.attributesOfItem(atPath: directory.path)
    let modified = (attributes?[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
    if modified < cutoff {
      try? FileManager.default.removeItem(at: directory)
    }
  }

  private static func isSafeIdentifier(_ id: String) -> Bool {
    !id.isEmpty && id.allSatisfy { $0.isHexDigit || $0 == "-" }
  }
}

/// The destinations the app offers the Share Extension.
///
/// The app rewrites this file whenever its channel list changes, so the sheet
/// can show the same channels, in the same order, without the app running.
/// Labels are presentation only; the app re-validates the chosen channel
/// against its live channel list before composing anything.
enum BuzzShareTargets {
  static let fileName = "share-targets.json"
  static let currentVersion = 1

  struct Community: Codable, Equatable, Identifiable {
    let id: String
    let name: String
  }

  struct Target: Codable, Equatable, Identifiable {
    let communityID: String
    let channelID: String
    let label: String
    let isDM: Bool
    let lastMessageAt: TimeInterval?

    var id: String { "\(communityID)/\(channelID)" }
  }

  struct File: Codable, Equatable {
    var version: Int = currentVersion
    var communities: [Community]
    var targets: [Target]
    var updatedAt: TimeInterval

    static let empty = File(communities: [], targets: [], updatedAt: 0)
  }

  static func fileURL(appGroupIdentifier: String?) -> URL? {
    guard let container = BuzzShareInbox.containerURL(appGroupIdentifier: appGroupIdentifier)
    else { return nil }
    let directory =
      container
      .appendingPathComponent("Library", isDirectory: true)
      .appendingPathComponent("Application Support", isDirectory: true)
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory.appendingPathComponent(fileName)
  }

  static func read(appGroupIdentifier: String?) -> File {
    guard let url = fileURL(appGroupIdentifier: appGroupIdentifier),
      let data = try? Data(contentsOf: url),
      let file = try? JSONDecoder().decode(File.self, from: data),
      file.version == currentVersion
    else { return .empty }
    return file
  }

  /// Replaces one community's targets, keeps the others, and drops targets of
  /// communities the app no longer has.
  static func merge(
    into existing: File,
    communities: [Community],
    communityID: String,
    targets: [Target]
  ) -> File {
    let known = Set(communities.map(\.id))
    let kept = existing.targets.filter { $0.communityID != communityID && known.contains($0.communityID) }
    return File(
      communities: communities,
      targets: kept + targets.filter { $0.communityID == communityID },
      updatedAt: Date().timeIntervalSince1970
    )
  }

  static func write(_ file: File, appGroupIdentifier: String?) throws {
    guard let url = fileURL(appGroupIdentifier: appGroupIdentifier) else {
      throw CocoaError(.fileNoSuchFile)
    }
    let data = try JSONEncoder().encode(file)
    try data.write(to: url, options: .atomic)
  }
}
