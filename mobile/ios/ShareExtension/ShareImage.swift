import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Readies a shared photo for upload with the same result as the app's
/// composer (`_prepareUploadImage` in media_upload.dart and MediaSanitizer):
/// HEIC and other formats become JPEG, WebP becomes PNG, orientation is
/// applied to the pixels, colors are sRGB and metadata is scrubbed. The pixels
/// go through ShareImageEncoder, a band at a time, to stay within the share
/// extension's memory limit.
enum ShareImage {
  static func prepare(_ provider: NSItemProvider) async throws -> SharedContent.Attachment {
    let offered = provider.registeredTypeIdentifiers.compactMap(UTType.init)
    // A GIF first: Photos can also offer a still JPEG of it.
    let type =
      offered.first { $0.conforms(to: .gif) }
      ?? offered.first { $0.conforms(to: .jpeg) }
      ?? offered.first { $0.conforms(to: .heic) }
      ?? offered.first { $0.conforms(to: .image) }
      ?? .image
    let (source, _) = try await loadFile(provider, type: type)
    var temporary = [source]
    defer { temporary.forEach { try? FileManager.default.removeItem(at: $0) } }
    let handle = try FileHandle(forReadingFrom: source)
    let head = try handle.read(upToCount: 16) ?? Data()
    try? handle.close()
    let mime = detectMime(head)
    if mime == "image/gif" {
      // Sent as is; the share screen strips its metadata with the app's
      // Dart sanitizer before it uploads, as the composer does.
      temporary.removeAll()
      let size = (try? source.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
      guard size <= SharedContent.maximumImageBytes else {
        try? FileManager.default.removeItem(at: source)
        throw ShareError.message("This GIF is too large to send (over 50 MB).")
      }
      return SharedContent.Attachment(kind: .gif, url: source, mime: mime!, size: size)
    }
    let png = mime == "image/png" || mime == "image/webp"
    var input = source
    if mime != "image/jpeg" && !png {
      // HEIC and the rest: transcode to JPEG first, which ImageIO streams,
      // so the banded pass below decodes a format it can read piecemeal.
      input = try transcodeToJpeg(source)
      temporary.append(input)
    }
    let output = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString)
      .appendingPathExtension(png ? "png" : "jpg")
    do {
      try ShareImageEncoder.encode(source: input, to: output, as: png ? .png : .jpeg)
      let encoded = try Data(contentsOf: output)
      let scrubbed = png ? try MediaSanitizer.scrubPng(encoded) : try MediaSanitizer.scrubJpeg(encoded)
      try scrubbed.write(to: output)
    } catch {
      NSLog("[BuzzShare] preparing photo failed: %@", String(describing: error))
      throw ShareError.message("We couldn't prepare this image for upload.")
    }
    let size = (try? output.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    guard size <= SharedContent.maximumImageBytes else {
      try? FileManager.default.removeItem(at: output)
      throw ShareError.message("This photo is too large to send (over 50 MB).")
    }
    return SharedContent.Attachment(kind: .image, url: output, mime: png ? "image/png" : "image/jpeg", size: size)
  }

  private static func transcodeToJpeg(_ source: URL) throws -> URL {
    let output = FileManager.default.temporaryDirectory
      .appendingPathComponent("transcoded-\(UUID().uuidString)").appendingPathExtension("jpg")
    guard
      let image = CGImageSourceCreateWithURL(source as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
      let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
    else { throw ShareError.message("We couldn't prepare this image for upload.") }
    CGImageDestinationAddImageFromSource(
      destination, image, 0, [kCGImageDestinationLossyCompressionQuality: 1.0] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else {
      throw ShareError.message("We couldn't prepare this image for upload.")
    }
    return output
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
  /// callback returns. Also returns the file's name.
  static func loadFile(_ provider: NSItemProvider, type: UTType) async throws -> (URL, String) {
    try await withCheckedThrowingContinuation { continuation in
      provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
        guard let url else {
          return continuation.resume(throwing: error ?? ShareError.message("Couldn't read the shared file."))
        }
        let copy = FileManager.default.temporaryDirectory
          .appendingPathComponent("source-\(UUID().uuidString)")
          .appendingPathExtension(url.pathExtension)
        do {
          try FileManager.default.copyItem(at: url, to: copy)
          continuation.resume(returning: (copy, url.lastPathComponent))
        } catch {
          continuation.resume(throwing: error)
        }
      }
    }
  }
}
