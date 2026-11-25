import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Re-encodes a photo the way the app's composer does (orientation applied to
/// the pixels, sRGB, no metadata) without holding the decoded image: the
/// output is drawn and encoded a band of rows at a time, and each band
/// decodes only the part of the source it covers. A share extension can't
/// afford the composer's full-size bitmaps.
enum ShareImageEncoder {
  enum Failure: Error { case unreadable, encode }

  /// Writes the re-encoded image to `output` as `type` (JPEG or PNG).
  static func encode(source url: URL, to output: URL, as type: UTType, bandRows: Int = 256) throws {
    guard
      let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
      let image = CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCache: false] as CFDictionary),
      let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
      let sRGB = CGColorSpace(name: CGColorSpace.sRGB)
    else { throw Failure.unreadable }
    let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
    let (w, h) = (CGFloat(image.width), CGFloat(image.height))
    let transform = displayTransform(orientation, width: w, height: h)
    let swapped = orientation >= 5 && orientation <= 8
    let (outW, outH) = swapped ? (image.height, image.width) : (image.width, image.height)
    let rowBytes = outW * 4
    // PNG keeps transparency, as the composer's PNG path does; JPEG has none.
    let alpha: CGImageAlphaInfo = type.conforms(to: .png) ? .premultipliedLast : .noneSkipLast
    let band = try BandRenderer(
      image: image, transform: transform, outW: outW, outH: outH, bandRows: bandRows, space: sRGB,
      alpha: alpha)

    var callbacks = CGDataProviderSequentialCallbacks(
      version: 0,
      getBytes: { info, buffer, count in
        let band = Unmanaged<BandRenderer>.fromOpaque(info!).takeUnretainedValue()
        return band.read(into: buffer, count: count)
      },
      skipForward: { info, count in
        let band = Unmanaged<BandRenderer>.fromOpaque(info!).takeUnretainedValue()
        return band.skip(Int(count))
      },
      rewind: { info in Unmanaged<BandRenderer>.fromOpaque(info!).takeUnretainedValue().rewind() },
      releaseInfo: { info in Unmanaged<BandRenderer>.fromOpaque(info!).release() })
    guard
      let provider = CGDataProvider(
        sequentialInfo: Unmanaged.passRetained(band).toOpaque(), callbacks: &callbacks),
      let rotated = CGImage(
        width: outW, height: outH, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: rowBytes,
        space: sRGB, bitmapInfo: CGBitmapInfo(rawValue: alpha.rawValue),
        provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent),
      let destination = CGImageDestinationCreateWithURL(output as CFURL, type.identifier as CFString, 1, nil)
    else { throw Failure.encode }
    CGImageDestinationAddImage(
      destination, rotated, [kCGImageDestinationLossyCompressionQuality: 1.0] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else { throw Failure.encode }
  }

  /// Maps the stored image's rect (Core Graphics coordinates, y up) onto the
  /// upright image, per the EXIF orientation.
  static func displayTransform(_ orientation: Int, width w: CGFloat, height h: CGFloat) -> CGAffineTransform {
    switch orientation {
    case 2: return CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: w, ty: 0)
    case 3: return CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: w, ty: h)
    case 4: return CGAffineTransform(a: 1, b: 0, c: 0, d: -1, tx: 0, ty: h)
    case 5: return CGAffineTransform(a: 0, b: -1, c: -1, d: 0, tx: h, ty: w)
    case 6: return CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: w)
    case 7: return CGAffineTransform(a: 0, b: 1, c: 1, d: 0, tx: 0, ty: 0)
    case 8: return CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: h, ty: 0)
    default: return .identity
    }
  }
}

/// Produces the upright image's rows top to bottom, rendering one band at a
/// time from just the part of the source that band shows.
private final class BandRenderer {
  private let image: CGImage
  private let transform: CGAffineTransform
  private let outW: Int, outH: Int, bandRows: Int
  private let context: CGContext
  private var bandStart = -1  // first output row held in `context`
  private var offset = 0  // bytes of output already handed out

  init(
    image: CGImage, transform: CGAffineTransform, outW: Int, outH: Int, bandRows: Int,
    space: CGColorSpace, alpha: CGImageAlphaInfo
  ) throws {
    guard
      let context = CGContext(
        data: nil, width: outW, height: bandRows, bitsPerComponent: 8, bytesPerRow: outW * 4,
        space: space, bitmapInfo: alpha.rawValue)
    else { throw ShareImageEncoder.Failure.encode }
    context.interpolationQuality = .none
    (self.image, self.transform, self.outW, self.outH, self.bandRows, self.context) =
      (image, transform, outW, outH, bandRows, context)
  }

  private var total: Int { outW * 4 * outH }

  func read(into buffer: UnsafeMutableRawPointer, count: Int) -> Int {
    var written = 0
    let rowBytes = outW * 4
    while written < count, offset < total {
      let row = offset / rowBytes
      let start = row - row % bandRows
      if start != bandStart { render(from: start) }
      let bandEnd = min(start + bandRows, outH) * rowBytes
      let n = min(count - written, bandEnd - offset)
      memcpy(buffer + written, context.data! + (offset - start * rowBytes), n)
      written += n
      offset += n
    }
    return written
  }

  func skip(_ count: Int) -> off_t {
    let n = min(count, total - offset)
    offset += n
    return off_t(n)
  }

  func rewind() { offset = 0 }

  /// Draws output rows `start ..< start + bandRows` into `context`.
  private func render(from start: Int) {
    bandStart = start
    let rows = min(bandRows, outH - start)
    // The band in output coordinates (y up), and the source rect it shows.
    let bandRect = CGRect(x: 0, y: outH - start - rows, width: outW, height: rows)
    let sourceRect = bandRect.applying(transform.inverted()).integral
      .intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
    let cropRect = CGRect(
      x: sourceRect.minX, y: CGFloat(image.height) - sourceRect.maxY,
      width: sourceRect.width, height: sourceRect.height)
    guard let piece = image.cropping(to: cropRect) else { return }
    context.saveGState()
    context.clear(CGRect(x: 0, y: 0, width: outW, height: bandRows))
    // Context row 0 (top) is output row `start`; the context's y-up origin
    // is the band's bottom edge.
    context.translateBy(x: 0, y: CGFloat(bandRows - rows) - bandRect.minY)
    context.concatenate(transform)
    context.draw(piece, in: sourceRect)
    context.restoreGState()
  }
}
