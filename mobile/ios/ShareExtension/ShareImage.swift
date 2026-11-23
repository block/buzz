import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Readies a shared photo for upload without scaling or re-encoding it.
enum ShareImage {
  static func prepare(_ provider: NSItemProvider) async throws -> SharedContent.Image {
    // Prefer a JPEG when the source offers one (Photos often does), since every
    // client can show it; otherwise send what was shared as is.
    let offered = provider.registeredTypeIdentifiers.compactMap(UTType.init)
    let type =
      offered.first { $0.conforms(to: .jpeg) }
      ?? offered.first { $0.conforms(to: .heic) }
      ?? offered.first { $0.conforms(to: .image) }
      ?? .image
    NSLog("[BuzzShare] sending image as %@", type.identifier)
    let source = try await loadFile(provider, type: type)
    let output = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString)
      .appendingPathExtension(source.pathExtension.isEmpty ? (type.preferredFilenameExtension ?? "img") : source.pathExtension)
    try stripLocation(from: source, to: output)
    let size = (try? output.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    guard size <= SharedContent.maximumImageBytes else {
      throw ShareError.message("This photo is too large to send (over 50 MB).")
    }
    let mime = UTType(filenameExtension: output.pathExtension)?.preferredMIMEType
      ?? type.preferredMIMEType ?? "application/octet-stream"
    return SharedContent.Image(url: output, mime: mime, size: size)
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

  /// A lossless copy without GPS metadata. ImageIO rewrites only the
  /// metadata; it never decodes the pixels.
  private static func stripLocation(from source: URL, to output: URL) throws {
    guard let image = CGImageSourceCreateWithURL(source as CFURL, nil),
      let type = CGImageSourceGetType(image),
      let destination = CGImageDestinationCreateWithURL(output as CFURL, type, 1, nil)
    else { throw ShareError.message("Couldn't read the photo.") }
    let options: [CFString: Any] = [kCGImageMetadataShouldExcludeGPS: true]
    var error: Unmanaged<CFError>?
    guard CGImageDestinationCopyImageSource(destination, image, options as CFDictionary, &error) else {
      throw error?.takeRetainedValue() ?? ShareError.message("Couldn't remove the photo's location.")
    }
    try? FileManager.default.removeItem(at: source)
  }
}
