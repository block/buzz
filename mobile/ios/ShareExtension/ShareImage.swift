import Foundation
import UIKit
import UniformTypeIdentifiers

/// Readies a shared photo for upload the way the app's composer does: the
/// same native re-encode (HEIC to JPEG, sRGB) and metadata scrub as the
/// `sanitizeImageForUpload` and `transcodeImageToJpeg` calls in AppDelegate.
enum ShareImage {
  static func prepare(_ provider: NSItemProvider) async throws -> SharedContent.Image {
    let offered = provider.registeredTypeIdentifiers.compactMap(UTType.init)
    let type =
      offered.first { $0.conforms(to: .jpeg) }
      ?? offered.first { $0.conforms(to: .heic) }
      ?? offered.first { $0.conforms(to: .image) }
      ?? .image
    let source = try await loadFile(provider, type: type)
    defer { try? FileManager.default.removeItem(at: source) }
    let bytes = try Data(contentsOf: source)
    let (encoded, mime, ext) = try sanitize(bytes)
    guard encoded.count <= SharedContent.maximumImageBytes else {
      throw ShareError.message("This photo is too large to send (over 50 MB).")
    }
    let output = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString)
      .appendingPathExtension(ext)
    try encoded.write(to: output)
    return SharedContent.Image(url: output, mime: mime, size: encoded.count)
  }

  /// Mirrors `_prepareUploadImage` in the app's media_upload.dart.
  private static func sanitize(_ bytes: Data) throws -> (Data, String, String) {
    func fail() -> Error { ShareError.message("We couldn't prepare this image for upload.") }
    let mime = detectMime(bytes)
    if mime == "image/gif" {
      // The app sanitizes animated images in Dart; not wired up here yet.
      throw ShareError.message("GIFs can't be shared from here yet.")
    }
    guard let image = UIImage(data: bytes) else { throw fail() }
    let output: Data?
    switch mime {
    case "image/jpeg", "image/png", "image/webp":
      output = try MediaSanitizer.sanitizeImage(image, mimeType: mime!)
    default:
      // HEIC and anything else UIKit can decode goes to JPEG, as the app does.
      output = try MediaSanitizer.encodeJpeg(image)
    }
    guard let output, !output.isEmpty else { throw fail() }
    return output.starts(with: [0x89, 0x50, 0x4E, 0x47])
      ? (output, "image/png", "png") : (output, "image/jpeg", "jpg")
  }

  private static func detectMime(_ bytes: Data) -> String? {
    if bytes.starts(with: [0xFF, 0xD8, 0xFF]) { return "image/jpeg" }
    if bytes.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) { return "image/png" }
    if bytes.starts(with: Array("GIF8".utf8)) { return "image/gif" }
    if bytes.count >= 12, bytes.prefix(4).elementsEqual(Array("RIFF".utf8)),
      bytes[bytes.startIndex + 8..<bytes.startIndex + 12].elementsEqual(Array("WEBP".utf8))
    {
      return "image/webp"
    }
    return nil
  }

  /// Copies the file the provider hands out, which it deletes when the
  /// callback returns.
  private static func loadFile(_ provider: NSItemProvider, type: UTType) async throws -> URL {
    try await withCheckedThrowingContinuation { continuation in
      provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
        guard let url else {
          return continuation.resume(throwing: error ?? ShareError.message("Couldn't read the photo."))
        }
        let copy = FileManager.default.temporaryDirectory
          .appendingPathComponent("source-\(UUID().uuidString)")
          .appendingPathExtension(url.pathExtension)
        do {
          try FileManager.default.copyItem(at: url, to: copy)
          continuation.resume(returning: copy)
        } catch {
          continuation.resume(throwing: error)
        }
      }
    }
  }
}
