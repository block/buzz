import Foundation
import SwiftDraw

/// Validates bounded SVG input before passing it to the native renderer.
enum BuzzNotificationSVG {
  static func decode(_ data: Data) throws -> Data {
    try validate(data)
    guard let svg = SVG(data: data), svg.size.width.isFinite, svg.size.height.isFinite,
      svg.size.width > 0, svg.size.height > 0,
      svg.size.width <= 16384, svg.size.height <= 16384
    else { throw BuzzNotificationCacheError.invalidRecord }
    let factor = min(1, 320 / max(svg.size.width, svg.size.height))
    let png = try svg.sized(
      CGSize(width: svg.size.width * factor, height: svg.size.height * factor)
    ).pngData(scale: 1)
    guard png.count <= BuzzNotificationCache.maximumAvatarBytes else {
      throw BuzzNotificationCacheError.oversizedRecord
    }
    return png
  }

  static func validate(_ data: Data) throws {
    guard data.count <= 128 * 1024, let text = String(data: data, encoding: .utf8),
      !text.lowercased().contains("<!doctype"), !text.lowercased().contains("<!entity"),
      !text.lowercased().contains("@font-face"), !text.lowercased().contains("@import")
    else { throw BuzzNotificationCacheError.invalidRecord }
    let validator = Validator()
    let parser = XMLParser(data: data)
    parser.shouldResolveExternalEntities = false
    parser.delegate = validator
    guard parser.parse(), validator.valid, validator.stack.isEmpty,
      validator.nodes.first?.name == "svg"
    else { throw BuzzNotificationCacheError.invalidRecord }
    try validator.checkReferences()
  }

  private final class Validator: NSObject, XMLParserDelegate {
    struct Node {
      let name: String
      var children: [Int] = []
      var references: [String] = []
    }
    var nodes: [Node] = []
    var stack: [Int] = []
    var identifiers: [String: Int] = [:]
    var valid = true
    private var textBytes = 0
    private var styleText = ""

    func parser(
      _ parser: XMLParser, didStartElement name: String, namespaceURI: String?,
      qualifiedName: String?, attributes: [String: String]
    ) {
      guard nodes.count < 512, stack.count < 32,
        !["font", "font-face", "foreignObject", "script"].contains(name),
        attributes.values.allSatisfy({ $0.utf8.count <= 32768 })
      else {
        reject(parser)
        return
      }
      let index = nodes.count
      nodes.append(Node(name: name))
      if let parent = stack.last { nodes[parent].children.append(index) }
      stack.append(index)
      if let id = attributes["id"] {
        guard identifiers[id] == nil else {
          reject(parser)
          return
        }
        identifiers[id] = index
      }
      for (key, value) in attributes {
        if key == "href" || key == "xlink:href" {
          if value.hasPrefix("#") {
            nodes[index].references.append(String(value.dropFirst()))
          } else if name == "image", value.hasPrefix("data:image/") {
            // Validate compressed raster bounds before SwiftDraw expands it.
            guard let comma = value.firstIndex(of: ","),
              ["data:image/png;base64", "data:image/jpeg;base64", "data:image/webp;base64"]
                .contains(String(value[..<comma])),
              let bytes = Data(base64Encoded: String(value[value.index(after: comma)...])),
              (try? BuzzNotificationAvatarLoader.decode(bytes)) != nil
            else {
              reject(parser)
              return
            }
          } else {
            reject(parser)
            return
          }
        }
        if value.contains("url(") {
          guard
            let expression = try? NSRegularExpression(
              pattern: #"url\(\s*['"]?#([^\s)'" ]+)['"]?\s*\)"#)
          else {
            reject(parser)
            return
          }
          let range = NSRange(value.startIndex..., in: value)
          let matches = expression.matches(in: value, range: range)
          guard matches.count == value.components(separatedBy: "url(").count - 1 else {
            reject(parser)
            return
          }
          for match in matches {
            guard let range = Range(match.range(at: 1), in: value) else {
              reject(parser)
              return
            }
            nodes[index].references.append(String(value[range]))
          }
        }
      }
    }

    func parser(
      _ parser: XMLParser, didEndElement: String, namespaceURI: String?, qualifiedName: String?
    ) {
      if didEndElement == "style" {
        let css = styleText.lowercased()
        // CSS escapes can hide resource references. Reject them before parsing.
        if css.contains("url(") || css.contains("@") || css.contains("\\") { reject(parser) }
        styleText = ""
      }
      if !stack.isEmpty { stack.removeLast() }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) {
      textBytes += string.utf8.count
      if stack.last.map({ nodes[$0].name == "style" }) == true { styleText += string }
      if textBytes > 16384 { reject(parser) }
    }

    func parser(_ parser: XMLParser, foundCDATA data: Data) {
      guard let string = String(data: data, encoding: .utf8) else {
        reject(parser)
        return
      }
      self.parser(parser, foundCharacters: string)
    }

    private func reject(_ parser: XMLParser) {
      valid = false
      parser.abortParsing()
    }

    func checkReferences() throws {
      var budget = 4096
      func visit(_ index: Int, ancestors: Set<Int>) throws {
        guard !ancestors.contains(index), budget > 0, ancestors.count < 32 else {
          throw BuzzNotificationCacheError.oversizedRecord
        }
        budget -= 1
        var ancestors = ancestors
        ancestors.insert(index)
        for child in nodes[index].children { try visit(child, ancestors: ancestors) }
        for reference in nodes[index].references {
          guard let target = identifiers[reference] else {
            throw BuzzNotificationCacheError.invalidRecord
          }
          try visit(target, ancestors: ancestors)
        }
      }
      try visit(0, ancestors: [])
    }
  }
}
