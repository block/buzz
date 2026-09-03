import Foundation

enum MP4Canonicalizer {
  static func neutralizeSampleDependencyBoxes(at url: URL) throws {
    var data = try Data(contentsOf: url)
    try neutralizeSampleDependencyBoxes(in: &data, start: 0, end: data.count)
    try data.write(to: url, options: .atomic)
  }

  private static func neutralizeSampleDependencyBoxes(
    in data: inout Data,
    start: Int,
    end: Int
  ) throws {
    let containers: Set<[UInt8]> = [
      Array("moov".utf8), Array("trak".utf8), Array("mdia".utf8),
      Array("minf".utf8), Array("stbl".utf8), Array("edts".utf8),
      Array("dinf".utf8), Array("sinf".utf8), Array("schi".utf8),
    ]
    let sampleDependencyType = Array("sdtp".utf8)
    let freeType = Array("free".utf8)
    var offset = start

    while offset < end {
      guard end - offset >= 8 else { throw invalidMp4BoxError() }
      let compactSize = Int(readBigEndianUInt32(data, at: offset))
      var headerSize = 8
      let boxSize: Int
      if compactSize == 1 {
        guard end - offset >= 16 else { throw invalidMp4BoxError() }
        let extendedSize = readBigEndianUInt64(data, at: offset + 8)
        guard extendedSize <= UInt64(Int.max) else { throw invalidMp4BoxError() }
        boxSize = Int(extendedSize)
        headerSize = 16
      } else if compactSize == 0 {
        boxSize = end - offset
      } else {
        boxSize = compactSize
      }

      guard boxSize >= headerSize, offset + boxSize <= end else {
        throw invalidMp4BoxError()
      }
      let type = Array(data[(offset + 4)..<(offset + 8)])
      if type == sampleDependencyType {
        data.replaceSubrange((offset + 4)..<(offset + 8), with: freeType)
      } else if containers.contains(type) {
        try neutralizeSampleDependencyBoxes(
          in: &data,
          start: offset + headerSize,
          end: offset + boxSize
        )
      }
      offset += boxSize
    }
  }

  private static func readBigEndianUInt32(_ data: Data, at offset: Int) -> UInt32 {
    data[offset..<(offset + 4)].reduce(0) { ($0 << 8) | UInt32($1) }
  }

  private static func readBigEndianUInt64(_ data: Data, at offset: Int) -> UInt64 {
    data[offset..<(offset + 8)].reduce(0) { ($0 << 8) | UInt64($1) }
  }

  private static func invalidMp4BoxError() -> NSError {
    NSError(
      domain: "BuzzVideoTranscode",
      code: 1,
      userInfo: [NSLocalizedDescriptionKey: "Invalid MP4 box structure."]
    )
  }
}
