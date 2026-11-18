import Flutter
import SwiftUI
import UIKit

struct NativeEmojiPickerAppearance {
  let surface: UIColor
  let control: UIColor
  let text: UIColor
  let secondaryText: UIColor
  let accent: UIColor
  let divider: UIColor
  let isDark: Bool

  init(arguments: [String: Any]) {
    surface = Self.color(arguments["surfaceColor"], fallback: .systemBackground)
    control = Self.color(
      arguments["controlColor"],
      fallback: .secondarySystemBackground
    )
    text = Self.color(arguments["textColor"], fallback: .label)
    secondaryText = Self.color(
      arguments["secondaryTextColor"],
      fallback: .secondaryLabel
    )
    accent = Self.color(arguments["accentColor"], fallback: .systemBlue)
    divider = Self.color(arguments["dividerColor"], fallback: .separator)
    isDark = arguments["isDark"] as? Bool ?? false
  }

  private static func color(_ raw: Any?, fallback: UIColor) -> UIColor {
    guard let value = (raw as? NSNumber)?.uint32Value else { return fallback }
    let alpha = CGFloat((value >> 24) & 0xFF) / 255
    let red = CGFloat((value >> 16) & 0xFF) / 255
    let green = CGFloat((value >> 8) & 0xFF) / 255
    let blue = CGFloat(value & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }
}

struct NativeEmojiItem: Identifiable, Hashable {
  let id: String
  let shortcode: String
  let value: String
  let name: String
  let keywords: [String]
  let glyph: String?
  let skinVariants: [String]
  let imageURL: URL?
}

struct NativeEmojiSkinTone: Identifiable {
  let id: Int
  let label: String
  let color: UIColor
}

let nativeEmojiSkinTones = [
  NativeEmojiSkinTone(
    id: 0,
    label: "Default",
    color: UIColor(red: 1, green: 0.788, blue: 0.227, alpha: 1)
  ),
  NativeEmojiSkinTone(
    id: 1,
    label: "Light",
    color: UIColor(red: 1, green: 0.855, blue: 0.718, alpha: 1)
  ),
  NativeEmojiSkinTone(
    id: 2,
    label: "Medium-light",
    color: UIColor(red: 0.906, green: 0.725, blue: 0.561, alpha: 1)
  ),
  NativeEmojiSkinTone(
    id: 3,
    label: "Medium",
    color: UIColor(red: 0.784, green: 0.549, blue: 0.38, alpha: 1)
  ),
  NativeEmojiSkinTone(
    id: 4,
    label: "Medium-dark",
    color: UIColor(red: 0.643, green: 0.38, blue: 0.204, alpha: 1)
  ),
  NativeEmojiSkinTone(
    id: 5,
    label: "Dark",
    color: UIColor(red: 0.365, green: 0.267, blue: 0.216, alpha: 1)
  ),
]

func validNativeEmojiSkinTone(_ value: Int) -> Int {
  nativeEmojiSkinTones.indices.contains(value) ? value : 0
}

struct NativeEmojiSection: Identifiable {
  let id: String
  let title: String
  let systemImage: String
  let items: [NativeEmojiItem]
}

struct NativeEmojiPickerData {
  let sections: [NativeEmojiSection]
  let standardItems: [NativeEmojiItem]
  let customItems: [NativeEmojiItem]
}

enum NativeEmojiPickerDataLoader {
  static let assetPath = "assets/emoji/emoji-data.json"

  static func load(arguments: [String: Any]) -> NativeEmojiPickerData? {
    let key = FlutterDartProject.lookupKey(forAsset: assetPath)
    let url = Bundle.main.bundleURL.appendingPathComponent(key)
    guard let data = try? Data(contentsOf: url) else { return nil }
    return parse(data: data, arguments: arguments)
  }

  static func parse(
    data: Data,
    arguments: [String: Any]
  ) -> NativeEmojiPickerData? {
    guard
      let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let rawCategories = root["categories"] as? [[String: Any]],
      let rawEmoji = root["emoji"] as? [String: Any]
    else {
      return nil
    }

    var sections: [NativeEmojiSection] = []
    var standardItems: [NativeEmojiItem] = []
    var byValue: [String: NativeEmojiItem] = [:]

    for category in rawCategories {
      guard
        let categoryID = category["id"] as? String,
        let emojiIDs = category["emoji"] as? [String]
      else {
        continue
      }

      var items: [NativeEmojiItem] = []
      for emojiID in emojiIDs {
        guard let record = rawEmoji[emojiID] as? [String: Any] else { continue }
        let name = record["n"] as? String ?? emojiID
        let keywords = record["k"] as? [String] ?? []
        let glyphs: [String]
        if let values = record["u"] as? [String] {
          glyphs = values
        } else if let value = record["u"] as? String {
          glyphs = [value]
        } else {
          glyphs = []
        }

        guard let defaultGlyph = glyphs.first else { continue }
        let item = NativeEmojiItem(
          id: emojiID,
          shortcode: emojiID,
          value: defaultGlyph,
          name: name,
          keywords: keywords,
          glyph: defaultGlyph,
          skinVariants: glyphs,
          imageURL: nil
        )
        items.append(item)
        standardItems.append(item)
        for glyph in glyphs where byValue[glyph] == nil {
          byValue[glyph] = item
        }
      }

      sections.append(
        NativeEmojiSection(
          id: categoryID,
          title: categoryTitle(categoryID),
          systemImage: categorySymbol(categoryID),
          items: items
        )
      )
    }

    let rawCustomEmoji = arguments["customEmoji"] as? [[String: Any]] ?? []
    let customItems = rawCustomEmoji.compactMap { raw -> NativeEmojiItem? in
      guard
        let shortcode = raw["shortcode"] as? String,
        let urlString = raw["url"] as? String,
        let url = URL(string: urlString)
      else {
        return nil
      }
      return NativeEmojiItem(
        id: "custom-\(shortcode)",
        shortcode: shortcode,
        value: ":\(shortcode):",
        name: shortcode,
        keywords: [],
        glyph: nil,
        skinVariants: [],
        imageURL: url
      )
    }
    let customByValue = Dictionary(
      customItems.map { ($0.value, $0) },
      uniquingKeysWith: { first, _ in first }
    )

    let recentValues = arguments["recent"] as? [String] ?? []
    var seenRecentIDs: Set<String> = []
    let recentItems = recentValues.compactMap { value -> NativeEmojiItem? in
      guard let item = byValue[value] ?? customByValue[value] else { return nil }
      return seenRecentIDs.insert(item.id).inserted ? item : nil
    }
    if !recentItems.isEmpty {
      sections.insert(
        NativeEmojiSection(
          id: "frequent",
          title: "Frequently used",
          systemImage: "clock",
          items: recentItems
        ),
        at: 0
      )
    }

    if !customItems.isEmpty {
      sections.append(
        NativeEmojiSection(
          id: "custom",
          title: "Custom",
          systemImage: "sparkles",
          items: customItems
        )
      )
    }

    return NativeEmojiPickerData(
      sections: sections,
      standardItems: standardItems,
      customItems: customItems
    )
  }

  private static func categoryTitle(_ id: String) -> String {
    switch id {
    case "people": return "Smileys & People"
    case "nature": return "Animals & Nature"
    case "foods": return "Food & Drink"
    case "activity": return "Activity"
    case "places": return "Travel & Places"
    case "objects": return "Objects"
    case "symbols": return "Symbols"
    case "flags": return "Flags"
    default: return id.capitalized
    }
  }

  private static func categorySymbol(_ id: String) -> String {
    switch id {
    case "people": return "face.smiling"
    case "nature": return "leaf"
    case "foods": return "fork.knife"
    case "activity": return "figure.run"
    case "places": return "airplane"
    case "objects": return "lightbulb"
    case "symbols": return "heart"
    case "flags": return "flag"
    default: return "circle.grid.3x3"
    }
  }
}

private struct NativeEmojiSearchScore: Comparable {
  let tier: Int
  let detail: Int
  let length: Int
  let code: String

  static func < (lhs: Self, rhs: Self) -> Bool {
    if lhs.tier != rhs.tier { return lhs.tier < rhs.tier }
    if lhs.detail != rhs.detail { return lhs.detail < rhs.detail }
    if lhs.length != rhs.length { return lhs.length < rhs.length }
    return lhs.code < rhs.code
  }
}

enum NativeEmojiSearch {
  static func results(
    query: String,
    items: [NativeEmojiItem]
  ) -> [NativeEmojiItem] {
    items.compactMap { item -> (NativeEmojiItem, NativeEmojiSearchScore)? in
      guard let score = score(query: query, item: item) else { return nil }
      return (item, score)
    }
    .sorted { $0.1 < $1.1 }
    .map(\.0)
  }

  private static func score(
    query: String,
    item: NativeEmojiItem
  ) -> NativeEmojiSearchScore? {
    let normalizedQuery = collapse(query)
    guard !normalizedQuery.isEmpty else { return nil }
    let code = item.shortcode.lowercased()
    let normalizedCode = collapse(code)

    if normalizedCode == normalizedQuery {
      return makeScore(tier: 0, detail: 0, code: code)
    }
    if normalizedCode.hasPrefix(normalizedQuery) {
      return makeScore(tier: 1, detail: 0, code: code)
    }

    let words = ([item.name] + item.keywords)
      .flatMap { $0.lowercased().split(whereSeparator: { " _-".contains($0) }) }
      .map(String.init)
    if let index = words.firstIndex(where: { $0.hasPrefix(query.lowercased()) }) {
      return makeScore(tier: 2, detail: index, code: code)
    }
    if let range = normalizedCode.range(of: normalizedQuery) {
      return makeScore(
        tier: 3,
        detail: normalizedCode.distance(from: normalizedCode.startIndex, to: range.lowerBound),
        code: code
      )
    }
    if let index = words.firstIndex(where: { $0.contains(query.lowercased()) }) {
      return makeScore(tier: 4, detail: index, code: code)
    }
    if let span = subsequenceSpan(normalizedQuery, in: normalizedCode) {
      return makeScore(tier: 5, detail: span, code: code)
    }
    return nil
  }

  private static func makeScore(
    tier: Int,
    detail: Int,
    code: String
  ) -> NativeEmojiSearchScore {
    NativeEmojiSearchScore(
      tier: tier,
      detail: detail,
      length: code.count,
      code: code
    )
  }

  private static func collapse(_ value: String) -> String {
    value.lowercased().filter { !":_ -\t\n".contains($0) }
  }

  private static func subsequenceSpan(_ query: String, in target: String) -> Int? {
    let queryCharacters = Array(query)
    guard !queryCharacters.isEmpty else { return nil }
    var queryIndex = 0
    var first: Int?
    var last = 0
    for (targetIndex, character) in target.enumerated() {
      guard character == queryCharacters[queryIndex] else { continue }
      if first == nil { first = targetIndex }
      last = targetIndex
      queryIndex += 1
      if queryIndex == queryCharacters.count {
        return last - (first ?? last)
      }
    }
    return nil
  }
}

/// The top offset of each pinned section header, keyed by section id, reported
/// up from the scrolling grid so the rail can follow manual scrolling.
///
/// The same stream also carries two viewport measurements under the reserved
/// keys below, so the tracker sees the section offsets and the viewport bounds
/// consistently in a single update. Section ids come from the emoji dataset and
/// never collide with these dotted reserved keys.
let nativeEmojiViewportBottomKey = "buzz.emoji.viewportBottom"
let nativeEmojiContentBottomKey = "buzz.emoji.contentBottom"

struct NativeEmojiSectionOffsetsKey: PreferenceKey {
  static let defaultValue: [String: CGFloat] = [:]

  static func reduce(
    value: inout [String: CGFloat],
    nextValue: () -> [String: CGFloat]
  ) {
    value.merge(nextValue(), uniquingKeysWith: { _, next in next })
  }
}

/// Keeps scroll-driven category selection out of the picker view's own state.
/// The category buttons observe this object directly, so updating the rail does
/// not invalidate and rebuild the scroll container underneath an active drag.
final class NativeEmojiCategorySelection: ObservableObject {
  @Published private(set) var selectedSectionID: String?

  init(initialSectionID: String?) {
    selectedSectionID = initialSectionID
  }

  func select(_ sectionID: String?) {
    guard sectionID != selectedSectionID else { return }
    selectedSectionID = sectionID
  }
}

/// Pure selection logic: the highlighted section is the last one whose header
/// has scrolled to or above the top of the viewport. Extracted so the
/// scroll-tracking behaviour can be unit-tested without a live scroll view.
enum NativeEmojiCategoryTracker {
  static func selectedSectionID(
    order: [String],
    offsets: [String: CGFloat],
    viewportTop: CGFloat,
    viewportBottom: CGFloat? = nil,
    contentBottom: CGFloat? = nil,
    currentSelection: String? = nil
  ) -> String? {
    // At the clamped bottom of an overflowing list, a final section shorter
    // than the viewport can never scroll its header to the top, so the
    // header-at-top rule would keep the preceding section highlighted while the
    // user is plainly viewing the last one. Detect that case first: the content
    // end is on screen (`contentBottom <= viewportBottom`) and the top has
    // scrolled away (`firstTop < viewportTop`, so the list really did overflow
    // rather than merely fitting). Highlight the last section then.
    if let viewportBottom,
      let contentBottom,
      contentBottom <= viewportBottom + 1,
      let firstID = order.first,
      let firstTop = offsets[firstID],
      firstTop < viewportTop,
      let lastID = order.last
    {
      return lastID
    }

    var selected: String?
    for id in order {
      guard let top = offsets[id] else { continue }
      // A small tolerance keeps the header that is flush with the top pinned as
      // selected rather than flickering to the next section.
      if top <= viewportTop + 1 {
        selected = id
      } else {
        break
      }
    }

    let candidate = selected ?? currentSelection ?? order.first
    guard
      let candidate,
      let currentSelection,
      let candidateIndex = order.firstIndex(of: candidate),
      let currentIndex = order.firstIndex(of: currentSelection),
      candidateIndex < currentIndex
    else {
      return candidate
    }

    // Pinned headers can briefly report competing or incomplete positions as
    // one section pushes another off the top. Once the next section is active,
    // retain it through that small boundary jitter. A real upward scroll moves
    // its header clearly back into the viewport and then releases the latch.
    guard let currentTop = offsets[currentSelection] else {
      // A LazyVStack can discard the old pinned header after a fast upward
      // fling. Once an earlier header is a valid candidate, absence of the old
      // header is evidence to release the latch rather than retain it forever.
      return candidate
    }
    if currentTop <= viewportTop + 8 {
      return currentSelection
    }

    // The final, short section is selected from the content boundary rather
    // than its header. Keep that bottom selection stable until the content end
    // has visibly moved away from the viewport edge.
    if currentSelection == order.last {
      guard let viewportBottom, let contentBottom else {
        return currentSelection
      }
      if contentBottom <= viewportBottom + 8 {
        return currentSelection
      }
    }

    return candidate
  }
}
