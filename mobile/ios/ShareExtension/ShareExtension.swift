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
      if let attachment = shared.attachment {
        var map: [String: Any] = ["path": attachment.url.path, "mime": attachment.mime, "size": attachment.size]
        if let filename = attachment.filename { map["filename"] = filename }
        result[attachment.filename == nil ? "image" : "file"] = map
      }
      if let problem = shared.problem { result["problem"] = problem }
      let target = (extensionContext?.intent as? INSendMessageIntent)?.conversationIdentifier
      if let target { result["suggested"] = target }
      do {
        let relay = try ShareRelayClient.current()
        self.relay = relay
        result["community"] = [
          "id": relay.community.id, "name": relay.community.name, "pubkey": relay.pubkey,
          "relay": relay.baseURL.absoluteString,
        ]
        result["bestie"] = Self.bestie(pubkey: relay.pubkey)
        if let conversations = Self.conversations(pubkey: relay.pubkey) { result["conversations"] = conversations }
      } catch {
        result["signedOut"] = ShareError.describe(error)
      }
      return result
    case "query":
      guard let relay else { throw ShareError.notSignedIn }
      let filters = args["filters"] as? [[String: Any]] ?? []
      return try await relay.query(filters)
    case "upload":
      guard let relay else { throw ShareError.notSignedIn }
      guard let attachment = await content.value.attachment else {
        throw ShareError.message("Nothing to upload.")
      }
      return try await relay.upload(attachment)
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

  /// The conversations the app last saved for this identity, newest first.
  private static func conversations(pubkey: String) -> [Any]? {
    guard let url = ShareRelayClient.appGroupURL?.appendingPathComponent("Library/Caches/share-conversations.json"),
      let data = try? Data(contentsOf: url),
      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      json["pubkey"] as? String == pubkey
    else { return nil }
    return json["conversations"] as? [Any]
  }

  /// The Bestie the app last published for this identity, if any.
  private static func bestie(pubkey: String) -> [String: String]? {
    guard let url = ShareRelayClient.appGroupURL?.appendingPathComponent("Library/Caches/share-bestie.json"),
      let data = try? Data(contentsOf: url),
      let json = try? JSONSerialization.jsonObject(with: data) as? [String: String],
      json["pubkey"] == pubkey, let bestie = json["bestie"]
    else { return nil }
    return ["pubkey": bestie, "name": json["name"] ?? ""]
  }
}

enum ShareError: Error {
  case notSignedIn
  case message(String)

  static func describe(_ error: Error) -> String {
    switch error {
    case ShareError.notSignedIn: return "Sign in to Buzz and turn on notifications to share from here."
    case ShareError.message(let text): return text
    default: return error.localizedDescription
    }
  }
}

/// What the person shared: text and links, and at most one photo or file.
struct SharedContent {
  struct Attachment {
    let url: URL
    let mime: String
    let size: Int
    /// The file's name; nil for a photo.
    let filename: String?
  }

  var text: [String] = []
  var attachment: Attachment?
  var missed = 0
  var problem: String?

  /// The relay's default image limit.
  static let maximumImageBytes = 50 * 1024 * 1024
  /// The app's limit for file attachments.
  static let maximumFileBytes = 100 * 1024 * 1024

  static func load(from context: NSExtensionContext?) async -> SharedContent {
    var shared = SharedContent()
    let providers = ((context?.inputItems as? [NSExtensionItem]) ?? []).flatMap { $0.attachments ?? [] }
    for provider in providers {
      NSLog("[BuzzShare] offered types: %@", provider.registeredTypeIdentifiers.joined(separator: ", "))
      if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
        guard shared.attachment == nil else {
          shared.missed += 1
          continue
        }
        do { shared.attachment = try await ShareImage.prepare(provider) }
        catch {
          shared.missed += 1
          shared.problem = ShareError.describe(error)
        }
        continue
      }
      if provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier) {
        shared.missed += 1
        shared.problem = "Videos aren't supported in this prototype yet."
        continue
      }
      var loaded = false
      for type in [UTType.url, .plainText] where provider.hasItemConformingToTypeIdentifier(type.identifier) {
        let item = try? await provider.loadItem(forTypeIdentifier: type.identifier)
        // A shared document (a PDF, or a text file from Files) arrives as a
        // file URL; send the file itself, not its local path.
        if let url = item as? URL, !url.isFileURL { shared.text.append(url.absoluteString); loaded = true }
        else if let string = item as? String { shared.text.append(string); loaded = true }
        break
      }
      if loaded { continue }
      let offered = provider.registeredTypeIdentifiers.compactMap(UTType.init)
      guard let type = offered.first(where: { $0.conforms(to: .data) && $0 != .fileURL }) ?? offered.first(where: { $0.conforms(to: .item) }) else {
        shared.missed += 1
        continue
      }
      guard shared.attachment == nil else {
        shared.missed += 1
        continue
      }
      do { shared.attachment = try await loadDocument(provider, type: type) }
      catch {
        shared.missed += 1
        shared.problem = ShareError.describe(error)
      }
    }
    return shared
  }

  /// A file sent as is, like the composer's Files attachment: the bytes
  /// unchanged, typed application/octet-stream, with the file's name.
  private static func loadDocument(_ provider: NSItemProvider, type: UTType) async throws -> Attachment {
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
    return Attachment(url: url, mime: "application/octet-stream", size: size, filename: filename)
  }
}
