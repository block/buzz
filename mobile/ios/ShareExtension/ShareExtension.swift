import BuzzPushKit
import Flutter
import Intents
import UIKit
import UniformTypeIdentifiers

/// Prototype: shows Buzz's Flutter share screen inside the extension and sends
/// from this process, like a separate device that shares only the identity
/// (keychain and app group).
final class ShareViewController: UIViewController {
  private var engine: FlutterEngine?
  private var channel: FlutterMethodChannel?
  private lazy var content = Task { await SharedContent.load(from: self.extensionContext) }
  private var relay: ShareRelayClient?

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground
    _ = content
    startFlutter()
  }

  private func startFlutter() {
    // The extension runs the containing app's Dart code and engine, so
    // nothing is duplicated in the extension bundle.
    let appURL = Bundle.main.bundleURL.deletingLastPathComponent().deletingLastPathComponent()
    let dartBundle = Bundle(url: appURL.appendingPathComponent("Frameworks/App.framework"))
    let project = FlutterDartProject(precompiledDartBundle: dartBundle)
    let engine = FlutterEngine(name: "share", project: project, allowHeadlessExecution: false)
    engine.run(
      withEntrypoint: "shareExtensionMain",
      libraryURI: "package:buzz/share_extension/share_extension_main.dart")
    let channel = FlutterMethodChannel(name: "buzz/share_extension", binaryMessenger: engine.binaryMessenger)
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return result(nil) }
      Task { @MainActor in
        do { result(try await self.handle(call.method, call.arguments)) }
        catch { result(FlutterError(code: "failed", message: ShareError.describe(error), details: nil)) }
      }
    }
    let controller = FlutterViewController(engine: engine, nibName: nil, bundle: nil)
    addChild(controller)
    controller.view.frame = view.bounds
    controller.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(controller.view)
    controller.didMove(toParent: self)
    self.engine = engine
    self.channel = channel
  }

  @MainActor
  private func handle(_ method: String, _ arguments: Any?) async throws -> Any? {
    let args = arguments as? [String: Any] ?? [:]
    switch method {
    case "load":
      let shared = await content.value
      var result: [String: Any] = ["text": shared.text, "missed": shared.missed]
      result["attachments"] = shared.attachments.map(\.map)
      if let problem = shared.problem { result["problem"] = problem }
      let target = (extensionContext?.intent as? INSendMessageIntent)?.conversationIdentifier
      if let target { result["suggested"] = target }
      result["communities"] = ShareRelayClient.accounts().map { ["id": $0.id, "name": $0.name] }
      do {
        result.merge(try connect(nil)) { $1 }
      } catch {
        result["signedOut"] = ShareError.describe(error)
      }
      return result
    case "switchCommunity":
      return try connect(args["id"] as? String)
    case "query":
      guard let relay else { throw ShareError.notSignedIn }
      let filters = args["filters"] as? [[String: Any]] ?? []
      return try await relay.query(filters)
    case "upload":
      guard let relay else { throw ShareError.notSignedIn }
      let attachments = await content.value.attachments
      guard let index = args["index"] as? Int, attachments.indices.contains(index) else {
        throw ShareError.message("Nothing to upload.")
      }
      return try await relay.upload(attachments[index])
    case "publish":
      guard let relay else { throw ShareError.notSignedIn }
      return try await relay.publish(
        kind: args["kind"] as? Int ?? 9, content: args["content"] as? String ?? "",
        tags: args["tags"] as? [[String]] ?? [],
        createdAt: args["createdAt"] as? Int ?? Int(Date().timeIntervalSince1970))
    case "close":
      extensionContext?.completeRequest(returningItems: nil)
      return nil
    case "cancel":
      extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
      return nil
    default:
      return FlutterMethodNotImplemented
    }
  }

  /// Signs in to community `id` (the last used one when nil) and returns
  /// what the share screen shows for it.
  private func connect(_ id: String?) throws -> [String: Any] {
    let relay = try ShareRelayClient.current(id: id)
    self.relay = relay
    var result: [String: Any] = [
      "community": [
        "id": relay.community.id, "name": relay.community.name, "pubkey": relay.pubkey,
        "relay": relay.baseURL.absoluteString,
      ]
    ]
    result["bestie"] = Self.bestie(pubkey: relay.pubkey, community: relay.community.id)
    if let conversations = Self.conversations(pubkey: relay.pubkey, community: relay.community.id) {
      result["conversations"] = conversations
    }
    return result
  }

  /// The conversations the app last saved for this community, newest first.
  private static func conversations(pubkey: String, community: String) -> [Any]? {
    guard let url = ShareRelayClient.appGroupURL?.appendingPathComponent("Library/Caches/share-conversations.json"),
      let data = try? Data(contentsOf: url),
      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      json["pubkey"] as? String == pubkey, json["community"] as? String == community
    else { return nil }
    return json["conversations"] as? [Any]
  }

  /// The Bestie the app last published for this identity in this community,
  /// if any.
  private static func bestie(pubkey: String, community: String) -> [String: String]? {
    guard let url = ShareRelayClient.appGroupURL?.appendingPathComponent("Library/Caches/share-bestie.json"),
      let data = try? Data(contentsOf: url),
      let json = try? JSONSerialization.jsonObject(with: data) as? [String: String],
      json["pubkey"] == pubkey, json["community"] == community, let bestie = json["bestie"]
    else { return nil }
    return ["pubkey": bestie, "name": json["name"] ?? ""]
  }
}

enum ShareError: Error {
  case notSignedIn
  case message(String)

  static func describe(_ error: Error) -> String {
    switch error {
    case ShareError.notSignedIn: return "Open Buzz and sign in to share from here."
    case ShareError.message(let text): return text
    default: return error.localizedDescription
    }
  }
}

/// What the person shared: text and links, and up to
/// `maximumAttachments` photos, GIFs, videos and files.
struct SharedContent {
  enum Kind: String {
    case image, gif, video, file
  }

  struct Attachment {
    let kind: Kind
    let url: URL
    let mime: String
    let size: Int
    /// The file's name, for a file.
    var filename: String? = nil
    /// The poster frame, for a video.
    var poster: URL? = nil

    var map: [String: Any] {
      var map: [String: Any] = ["kind": kind.rawValue, "path": url.path, "mime": mime, "size": size]
      if let filename { map["filename"] = filename }
      if let poster { map["poster"] = poster.path }
      return map
    }
  }

  var text: [String] = []
  var attachments: [Attachment] = []
  var missed = 0
  var problem: String?

  /// As many attachments as the composer takes in one message.
  static let maximumAttachments = 10
  /// The relay's default image limit.
  static let maximumImageBytes = 50 * 1024 * 1024
  /// The app's limit for videos and file attachments.
  static let maximumFileBytes = 100 * 1024 * 1024

  static func load(from context: NSExtensionContext?) async -> SharedContent {
    var shared = SharedContent()
    let providers = ((context?.inputItems as? [NSExtensionItem]) ?? []).flatMap { $0.attachments ?? [] }
    for provider in providers {
      NSLog("[BuzzShare] offered types: %@", provider.registeredTypeIdentifiers.joined(separator: ", "))
      if let text = await loadText(provider) {
        shared.text.append(text)
        continue
      }
      guard shared.attachments.count < maximumAttachments else {
        shared.missed += 1
        continue
      }
      do {
        if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
          shared.attachments.append(try await ShareImage.prepare(provider))
        } else if provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier) {
          shared.attachments.append(try await loadVideo(provider))
        } else if let document = try await loadDocument(provider) {
          shared.attachments.append(document)
        } else {
          shared.missed += 1
        }
      } catch {
        shared.missed += 1
        shared.problem = ShareError.describe(error)
      }
    }
    return shared
  }

  /// A shared link or text. A shared document (a PDF, or a text file from
  /// Files) can also arrive as a file URL; that is sent as a file instead.
  private static func loadText(_ provider: NSItemProvider) async -> String? {
    if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier)
      || provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier)
    {
      return nil
    }
    for type in [UTType.url, .plainText] where provider.hasItemConformingToTypeIdentifier(type.identifier) {
      let item = try? await provider.loadItem(forTypeIdentifier: type.identifier)
      if let url = item as? URL, !url.isFileURL { return url.absoluteString }
      if let string = item as? String { return string }
      return nil
    }
    return nil
  }

  /// A video, re-encoded as the composer does (canonical H.264/AAC MP4, no
  /// metadata), with its poster frame.
  private static func loadVideo(_ provider: NSItemProvider) async throws -> Attachment {
    let offered = provider.registeredTypeIdentifiers.compactMap(UTType.init)
    let type = offered.first { $0.conforms(to: .movie) } ?? .movie
    let (source, _) = try await ShareImage.loadFile(provider, type: type)
    defer { try? FileManager.default.removeItem(at: source) }
    let video: URL = try await withCheckedThrowingContinuation { continuation in
      VideoTranscoder.transcode(source: source) { continuation.resume(with: $0) }
    }
    let size = (try? video.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    guard size <= maximumFileBytes else {
      try? FileManager.default.removeItem(at: video)
      throw ShareError.message("This video is too large to send (over 100 MB).")
    }
    // Best effort, as in the app: a video without a poster still sends.
    var poster: URL?
    if let jpeg = try? VideoTranscoder.poster(source: video) {
      let url = FileManager.default.temporaryDirectory.appendingPathComponent("poster-\(UUID().uuidString).jpg")
      if (try? jpeg.write(to: url)) != nil { poster = url }
    }
    return Attachment(kind: .video, url: video, mime: "video/mp4", size: size, poster: poster)
  }

  /// A file sent as is, like the composer's Files attachment: the bytes
  /// unchanged, typed application/octet-stream, with the file's name. Nil
  /// when the item isn't a file.
  private static func loadDocument(_ provider: NSItemProvider) async throws -> Attachment? {
    let offered = provider.registeredTypeIdentifiers.compactMap(UTType.init)
    guard
      let type = offered.first(where: { $0.conforms(to: .data) && $0 != .fileURL })
        ?? offered.first(where: { $0.conforms(to: .item) })
    else { return nil }
    let (url, name) = try await ShareImage.loadFile(provider, type: type)
    let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    guard size > 0 else {
      try? FileManager.default.removeItem(at: url)
      throw ShareError.message("This file is empty.")
    }
    guard size <= maximumFileBytes else {
      try? FileManager.default.removeItem(at: url)
      throw ShareError.message("This file is too large to send (over 100 MB).")
    }
    var filename = provider.suggestedName ?? name
    if let ext = type.preferredFilenameExtension, (filename as NSString).pathExtension.isEmpty {
      filename += ".\(ext)"
    }
    return Attachment(kind: .file, url: url, mime: "application/octet-stream", size: size, filename: filename)
  }
}
