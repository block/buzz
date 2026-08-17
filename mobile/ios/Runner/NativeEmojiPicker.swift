import Flutter
import ImageIO
import SwiftUI
import UIKit

private struct NativeEmojiPickerAppearance {
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

private struct NativeEmojiItem: Identifiable, Hashable {
  let id: String
  let shortcode: String
  let value: String
  let name: String
  let keywords: [String]
  let glyph: String?
  let skinVariants: [String]
  let imageURL: URL?
}

private struct NativeEmojiSkinTone: Identifiable {
  let id: Int
  let label: String
  let color: UIColor
}

private let nativeEmojiSkinTones = [
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

private func validNativeEmojiSkinTone(_ value: Int) -> Int {
  nativeEmojiSkinTones.indices.contains(value) ? value : 0
}

private struct NativeEmojiSection: Identifiable {
  let id: String
  let title: String
  let systemImage: String
  let items: [NativeEmojiItem]
}

private struct NativeEmojiPickerData {
  let sections: [NativeEmojiSection]
  let standardItems: [NativeEmojiItem]
  let customItems: [NativeEmojiItem]
}

private enum NativeEmojiPickerDataLoader {
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

private enum NativeEmojiSearch {
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
private struct NativeEmojiSectionOffsetsKey: PreferenceKey {
  static let defaultValue: [String: CGFloat] = [:]

  static func reduce(
    value: inout [String: CGFloat],
    nextValue: () -> [String: CGFloat]
  ) {
    value.merge(nextValue(), uniquingKeysWith: { _, next in next })
  }
}

/// Pure selection logic: the highlighted section is the last one whose header
/// has scrolled to or above the top of the viewport. Extracted so the
/// scroll-tracking behaviour can be unit-tested without a live scroll view.
enum NativeEmojiCategoryTracker {
  static func selectedSectionID(
    order: [String],
    offsets: [String: CGFloat],
    viewportTop: CGFloat
  ) -> String? {
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
    return selected ?? order.first
  }
}

private struct NativeEmojiPickerView: View {
  let data: NativeEmojiPickerData
  let appearance: NativeEmojiPickerAppearance
  let onSelect: (String) -> Void
  let onSkinToneChanged: (Int) -> Void
  let onClose: () -> Void

  @State private var query = ""
  @State private var selectedSectionID: String?
  @State private var selectedSkinTone: Int

  private let columns = Array(
    repeating: GridItem(.flexible(minimum: 36), spacing: 0),
    count: 8
  )

  private let sectionListSpace = "buzz.emoji.sectionList"

  init(
    data: NativeEmojiPickerData,
    appearance: NativeEmojiPickerAppearance,
    initialSkinTone: Int,
    onSelect: @escaping (String) -> Void,
    onSkinToneChanged: @escaping (Int) -> Void,
    onClose: @escaping () -> Void
  ) {
    self.data = data
    self.appearance = appearance
    self.onSelect = onSelect
    self.onSkinToneChanged = onSkinToneChanged
    self.onClose = onClose
    _selectedSkinTone = State(
      initialValue: validNativeEmojiSkinTone(initialSkinTone)
    )
  }

  var body: some View {
    ScrollViewReader { proxy in
      VStack(spacing: 0) {
        header
        if query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
          categoryRail(proxy)
        }
        Divider().overlay(Color(uiColor: appearance.divider))
        pickerContent
      }
      .background(Color(uiColor: appearance.surface))
      .onAppear {
        selectedSectionID = data.sections.first?.id
      }
    }
  }

  private var header: some View {
    HStack(spacing: 8) {
      HStack(spacing: 10) {
        Image(systemName: "magnifyingglass")
          .font(.system(size: 17, weight: .medium))
          .foregroundStyle(Color(uiColor: appearance.secondaryText))
        TextField("Search emoji", text: $query)
          .textInputAutocapitalization(.never)
          .autocorrectionDisabled(true)
          .submitLabel(.search)
          .foregroundStyle(Color(uiColor: appearance.text))
        if !query.isEmpty {
          Button {
            query = ""
          } label: {
            Image(systemName: "xmark.circle.fill")
              .foregroundStyle(Color(uiColor: appearance.secondaryText))
          }
          .buttonStyle(.plain)
          .accessibilityLabel("Clear search")
        }
      }
      .padding(.horizontal, 14)
      .frame(height: 44)
      .background(Color(uiColor: appearance.control), in: Capsule())
      .overlay {
        Capsule()
          .stroke(Color(uiColor: appearance.divider), lineWidth: 1)
      }

      Button(action: onClose) {
        Image(systemName: "xmark")
          .font(.system(size: 17, weight: .semibold))
          .foregroundStyle(Color(uiColor: appearance.text))
          .frame(width: 44, height: 44)
          .background(Color(uiColor: appearance.control), in: Circle())
      }
      .buttonStyle(.plain)
      .accessibilityLabel("Close sheet")
    }
    .padding(.horizontal, 16)
    .padding(.top, 16)
    .padding(.bottom, 8)
  }

  private func categoryRail(_ proxy: ScrollViewProxy) -> some View {
    HStack(spacing: 0) {
      ForEach(data.sections) { section in
        Button {
          selectedSectionID = section.id
          withAnimation(.easeOut(duration: 0.24)) {
            proxy.scrollTo("section-\(section.id)", anchor: .top)
          }
        } label: {
          Image(systemName: section.systemImage)
            .font(.system(size: 18, weight: .medium))
            .foregroundStyle(
              Color(
                uiColor: selectedSectionID == section.id
                  ? appearance.accent : appearance.secondaryText
              )
            )
            .frame(maxWidth: .infinity)
            .frame(height: 36)
            .background(
              selectedSectionID == section.id
                ? Color(uiColor: appearance.control) : Color.clear,
              in: Circle()
            )
        }
        .frame(maxWidth: .infinity)
        .buttonStyle(.plain)
        .accessibilityLabel(section.title)
        .accessibilityAddTraits(
          selectedSectionID == section.id ? .isSelected : []
        )
      }
      Divider()
        .frame(height: 24)
        .overlay(Color(uiColor: appearance.divider))
      skinToneSelector
        .frame(maxWidth: .infinity)
    }
    .padding(.horizontal, 16)
    .frame(height: 44)
  }

  private var skinToneSelector: some View {
    Menu {
      ForEach(nativeEmojiSkinTones) { tone in
        Button {
          selectedSkinTone = tone.id
          onSkinToneChanged(tone.id)
        } label: {
          Label {
            Text(tone.label)
          } icon: {
            Image(uiImage: skinTonePreviewImage(tone))
              .renderingMode(.original)
          }
        }
      }
    } label: {
      skinToneDot(nativeEmojiSkinTones[selectedSkinTone])
        .frame(maxWidth: .infinity)
        .frame(height: 36)
    }
    .buttonStyle(.plain)
    .accessibilityLabel("Skin tone")
  }

  private func skinToneDot(_ tone: NativeEmojiSkinTone) -> some View {
    Circle()
      .fill(Color(uiColor: tone.color))
      .frame(width: 16, height: 16)
      .overlay {
        Circle()
          .fill(
            LinearGradient(
              colors: [.white.opacity(0.2), .clear],
              startPoint: .top,
              endPoint: .bottom
            )
          )
          .blendMode(.overlay)
      }
      .overlay {
        Circle().stroke(.black.opacity(0.8), lineWidth: 1)
      }
  }

  private func skinTonePreviewImage(_ tone: NativeEmojiSkinTone) -> UIImage {
    let size = CGSize(width: 16, height: 16)
    return UIGraphicsImageRenderer(size: size).image { rendererContext in
      let context = rendererContext.cgContext
      let rect = CGRect(origin: .zero, size: size).insetBy(dx: 0.5, dy: 0.5)
      let circle = UIBezierPath(ovalIn: rect)

      tone.color.setFill()
      circle.fill()

      if let gradient = CGGradient(
        colorsSpace: CGColorSpaceCreateDeviceRGB(),
        colors: [
          UIColor.white.withAlphaComponent(0.2).cgColor,
          UIColor.clear.cgColor,
        ] as CFArray,
        locations: [0, 1]
      ) {
        context.saveGState()
        circle.addClip()
        context.setBlendMode(.overlay)
        context.drawLinearGradient(
          gradient,
          start: CGPoint(x: size.width / 2, y: 0),
          end: CGPoint(x: size.width / 2, y: size.height),
          options: []
        )
        context.restoreGState()
      }

      UIColor.black.withAlphaComponent(0.8).setStroke()
      circle.lineWidth = 1
      circle.stroke()
    }
  }

  @ViewBuilder
  private var pickerContent: some View {
    let trimmedQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmedQuery.isEmpty {
      sectionList(data.sections, tracksSelection: true)
    } else {
      let custom = NativeEmojiSearch.results(
        query: trimmedQuery,
        items: data.customItems
      )
      let standard = NativeEmojiSearch.results(
        query: trimmedQuery,
        items: data.standardItems
      )
      let sections = [
        NativeEmojiSection(
          id: "search-custom",
          title: "Custom",
          systemImage: "sparkles",
          items: custom
        ),
        NativeEmojiSection(
          id: "search-standard",
          title: "Emoji",
          systemImage: "face.smiling",
          items: standard
        ),
      ].filter { !$0.items.isEmpty }

      if sections.isEmpty {
        VStack(spacing: 10) {
          Image(systemName: "magnifyingglass")
            .font(.system(size: 28))
          Text("No emoji found").font(.body)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .foregroundStyle(Color(uiColor: appearance.secondaryText))
      } else {
        sectionList(sections, tracksSelection: false)
      }
    }
  }

  private func sectionList(
    _ sections: [NativeEmojiSection],
    tracksSelection: Bool
  ) -> some View {
    ScrollView {
      LazyVStack(spacing: 0, pinnedViews: [.sectionHeaders]) {
        ForEach(sections) { section in
          Section {
            LazyVGrid(columns: columns, spacing: 0) {
              ForEach(section.items) { item in
                emojiButton(item)
              }
            }
            .padding(.horizontal, 16)
          } header: {
            HStack {
              Text(section.title)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Color(uiColor: appearance.secondaryText))
              Spacer()
            }
            .padding(.horizontal, 16)
            .frame(height: 30)
            .background(Color(uiColor: appearance.surface))
            .background(sectionOffsetReporter(id: section.id))
            .id("section-\(section.id)")
          }
        }
      }
      .padding(.bottom, 8)
    }
    .coordinateSpace(name: sectionListSpace)
    .scrollDismissesKeyboard(.interactively)
    .onPreferenceChange(NativeEmojiSectionOffsetsKey.self) { offsets in
      guard tracksSelection else { return }
      selectedSectionID = NativeEmojiCategoryTracker.selectedSectionID(
        order: data.sections.map(\.id),
        offsets: offsets,
        viewportTop: 0
      )
    }
  }

  private func sectionOffsetReporter(id: String) -> some View {
    GeometryReader { geometry in
      Color.clear.preference(
        key: NativeEmojiSectionOffsetsKey.self,
        value: [id: geometry.frame(in: .named(sectionListSpace)).minY]
      )
    }
  }

  private func emojiButton(_ item: NativeEmojiItem) -> some View {
    let value = displayValue(for: item)
    return Button {
      onSelect(value)
    } label: {
      Group {
        if let url = item.imageURL {
          NativeEmojiRemoteImage(
            url: url,
            fallbackColor: appearance.secondaryText
          )
          .frame(width: 28, height: 28)
        } else {
          Text(value).font(.system(size: 28))
        }
      }
      .frame(maxWidth: .infinity)
      .frame(height: 44)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityLabel(item.name)
  }

  private func displayValue(for item: NativeEmojiItem) -> String {
    guard item.imageURL == nil else { return item.value }
    guard item.skinVariants.indices.contains(selectedSkinTone) else {
      return item.skinVariants.first ?? item.value
    }
    return item.skinVariants[selectedSkinTone]
  }
}

private struct NativeEmojiRemoteImage: View {
  let url: URL
  let fallbackColor: UIColor

  @State private var phase: Phase = .loading
  private static let maxDownloadBytes = 10 * 1024 * 1024
  private static let maxThumbnailPixels = 84

  private enum Phase {
    case loading
    case success(UIImage)
    case failure
  }

  var body: some View {
    Group {
      switch phase {
      case .loading:
        ProgressView().controlSize(.mini)
      case .success(let image):
        Image(uiImage: image).resizable().scaledToFit()
      case .failure:
        Image(systemName: "sparkles")
          .foregroundStyle(Color(uiColor: fallbackColor))
      }
    }
    .task(id: requestIdentity) {
      do {
        let requestHeaders = try await NativeEmojiPickerCoordinator.mediaHeaders(
          for: url
        )
        var request = URLRequest(url: url)
        for (name, value) in requestHeaders {
          request.setValue(value, forHTTPHeaderField: name)
        }
        let (bytes, response) = try await URLSession.shared.bytes(for: request)
        guard
          let httpResponse = response as? HTTPURLResponse,
          (200..<300).contains(httpResponse.statusCode)
        else {
          phase = .failure
          return
        }
        if let contentLength = httpResponse.value(forHTTPHeaderField: "Content-Length"),
          let byteCount = Int(contentLength),
          byteCount > Self.maxDownloadBytes
        {
          phase = .failure
          return
        }
        var data = Data()
        let expected = httpResponse.expectedContentLength
        if expected > 0 {
          data.reserveCapacity(
            Int(min(expected, Int64(Self.maxDownloadBytes)))
          )
        }
        for try await byte in bytes {
          guard data.count < Self.maxDownloadBytes else {
            phase = .failure
            return
          }
          data.append(byte)
        }
        guard let image = Self.thumbnail(from: data) else {
          phase = .failure
          return
        }
        phase = .success(image)
      } catch {
        if !Task.isCancelled { phase = .failure }
      }
    }
  }

  private static func thumbnail(from data: Data) -> UIImage? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
      return nil
    }
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceThumbnailMaxPixelSize: maxThumbnailPixels,
      kCGImageSourceShouldCacheImmediately: true,
    ]
    guard
      let image = CGImageSourceCreateThumbnailAtIndex(
        source,
        0,
        options as CFDictionary
      )
    else {
      return nil
    }
    return UIImage(cgImage: image)
  }

  private var requestIdentity: String {
    url.absoluteString
  }
}

private struct NativeEmojiMediaHeaderError: Error {}

final class NativeEmojiPickerCoordinator: NSObject,
  UIAdaptivePresentationControllerDelegate
{
  private let channel: FlutterMethodChannel
  private static weak var activeCoordinator: NativeEmojiPickerCoordinator?
  private weak var parentViewController: UIViewController?
  private weak var presentedController: UIViewController?
  private var didNotifyDismissal = false

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    channel = FlutterMethodChannel(
      name: "buzz/native_emoji_picker",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()
    Self.activeCoordinator = self
    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  static func mediaHeaders(for url: URL) async throws -> [String: String] {
    guard let channel = activeCoordinator?.channel else { return [:] }
    return try await withCheckedThrowingContinuation { continuation in
      channel.invokeMethod("mediaHeaders", arguments: url.absoluteString) { result in
        if result is FlutterError {
          continuation.resume(throwing: NativeEmojiMediaHeaderError())
          return
        }
        continuation.resume(returning: result as? [String: String] ?? [:])
      }
    }
  }

  private func handle(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    guard call.method == "present" else {
      result(FlutterMethodNotImplemented)
      return
    }
    guard let arguments = call.arguments as? [String: Any] else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected emoji-picker configuration.",
          details: nil
        )
      )
      return
    }

    DispatchQueue.main.async { [weak self] in
      result(self?.present(arguments: arguments) ?? false)
    }
  }

  @MainActor
  private func present(arguments: [String: Any]) -> Bool {
    // A sheet is already owned by an earlier caller; report busy rather than a
    // successful presentation so the caller does not treat this as its own.
    guard presentedController == nil else { return false }
    guard
      let data = NativeEmojiPickerDataLoader.load(arguments: arguments),
      let presenter = topViewController(
        from: parentViewController ?? activeWindowRootViewController()
      )
    else {
      return false
    }

    didNotifyDismissal = false
    let appearance = NativeEmojiPickerAppearance(arguments: arguments)
    let content = NativeEmojiPickerView(
      data: data,
      appearance: appearance,
      initialSkinTone: (arguments["skinTone"] as? NSNumber)?.intValue ?? 0,
      onSelect: { [weak self] emoji in self?.select(emoji) },
      onSkinToneChanged: { [weak self] value in
        self?.channel.invokeMethod("skinToneChanged", arguments: value)
      },
      onClose: { [weak self] in self?.dismiss() }
    )
    let controller = UIHostingController(rootView: content)
    controller.view.backgroundColor = appearance.surface
    controller.modalPresentationStyle = .pageSheet
    controller.overrideUserInterfaceStyle = appearance.isDark ? .dark : .light

    if let sheet = controller.sheetPresentationController {
      let compactID = UISheetPresentationController.Detent.Identifier(
        "buzz.emoji.compact"
      )
      let mediumID = UISheetPresentationController.Detent.Identifier(
        "buzz.emoji.medium"
      )
      sheet.detents = [
        .custom(identifier: compactID) { context in
          context.maximumDetentValue * 0.34
        },
        .custom(identifier: mediumID) { context in
          context.maximumDetentValue * 0.67
        },
        .large(),
      ]
      sheet.selectedDetentIdentifier = mediumID
      sheet.prefersGrabberVisible = true
      sheet.prefersScrollingExpandsWhenScrolledToEdge = false
      sheet.prefersEdgeAttachedInCompactHeight = false
      sheet.widthFollowsPreferredContentSizeWhenEdgeAttached = true
    }

    presentedController = controller
    presenter.present(controller, animated: true) { [weak self, weak controller] in
      controller?.presentationController?.delegate = self
    }
    return true
  }

  @MainActor
  private func select(_ emoji: String) {
    channel.invokeMethod("selected", arguments: emoji)
    dismiss()
  }

  @MainActor
  private func dismiss() {
    guard let controller = presentedController else {
      notifyDismissalIfNeeded()
      return
    }
    controller.dismiss(animated: true) { [weak self] in
      self?.notifyDismissalIfNeeded()
    }
  }

  func presentationControllerDidDismiss(
    _ presentationController: UIPresentationController
  ) {
    notifyDismissalIfNeeded()
  }

  @MainActor
  private func notifyDismissalIfNeeded() {
    guard !didNotifyDismissal else { return }
    didNotifyDismissal = true
    presentedController = nil
    channel.invokeMethod("dismissed", arguments: nil)
  }

  @MainActor
  private func activeWindowRootViewController() -> UIViewController? {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?
      .rootViewController
  }

  @MainActor
  private func topViewController(
    from viewController: UIViewController?
  ) -> UIViewController? {
    if let presented = viewController?.presentedViewController {
      return topViewController(from: presented)
    }
    if let navigation = viewController as? UINavigationController {
      return topViewController(from: navigation.visibleViewController)
    }
    if let tab = viewController as? UITabBarController {
      return topViewController(from: tab.selectedViewController)
    }
    return viewController
  }
}
