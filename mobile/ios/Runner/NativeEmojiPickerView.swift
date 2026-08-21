import ImageIO
import SwiftUI
import UIKit

struct NativeEmojiPickerView: View {
  let data: NativeEmojiPickerData
  let appearance: NativeEmojiPickerAppearance
  let onSelect: (String) -> Void
  let onSkinToneChanged: (Int) -> Void
  let onClose: () -> Void

  @State private var query = ""
  @State private var categorySelection: NativeEmojiCategorySelection
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
    _categorySelection = State(
      initialValue: NativeEmojiCategorySelection(
        initialSectionID: data.sections.first?.id
      )
    )
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
        NativeEmojiCategoryButton(
          section: section,
          appearance: appearance,
          selection: categorySelection
        ) {
          categorySelection.select(section.id)
          // Category navigation is a frequent shortcut. Keeping it immediate
          // means an in-progress proxy animation can never fight a finger drag.
          proxy.scrollTo("section-\(section.id)", anchor: .top)
        }
        .frame(maxWidth: .infinity)
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
      .background(contentBoundaryReporter())
    }
    .coordinateSpace(name: sectionListSpace)
    .background(viewportBoundaryReporter())
    .scrollDismissesKeyboard(.interactively)
    .onPreferenceChange(NativeEmojiSectionOffsetsKey.self) { offsets in
      guard tracksSelection else { return }
      categorySelection.select(
        NativeEmojiCategoryTracker.selectedSectionID(
          order: data.sections.map(\.id),
          offsets: offsets,
          viewportTop: 0,
          viewportBottom: offsets[nativeEmojiViewportBottomKey],
          contentBottom: offsets[nativeEmojiContentBottomKey],
          currentSelection: categorySelection.selectedSectionID
        )
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

  // The end of the scrolling content, relative to the viewport top. At the
  // clamped bottom of an overflowing list this converges on the viewport
  // height, which lets the tracker highlight a short final section that can
  // never scroll its own header to the top.
  private func contentBoundaryReporter() -> some View {
    GeometryReader { geometry in
      Color.clear.preference(
        key: NativeEmojiSectionOffsetsKey.self,
        value: [
          nativeEmojiContentBottomKey:
            geometry.frame(in: .named(sectionListSpace)).maxY
        ]
      )
    }
  }

  // The fixed viewport height, reported through the same preference stream so
  // it stays consistent with the section offsets in each update.
  private func viewportBoundaryReporter() -> some View {
    GeometryReader { geometry in
      Color.clear.preference(
        key: NativeEmojiSectionOffsetsKey.self,
        value: [nativeEmojiViewportBottomKey: geometry.size.height]
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

/// Observes only the rail selection. Keeping this in a leaf view prevents a
/// scroll-frame highlight update from rebuilding the picker grid itself.
private struct NativeEmojiCategoryButton: View {
  let section: NativeEmojiSection
  let appearance: NativeEmojiPickerAppearance
  @ObservedObject var selection: NativeEmojiCategorySelection
  let onSelect: () -> Void

  var body: some View {
    let isSelected = selection.selectedSectionID == section.id
    Button(action: onSelect) {
      Image(systemName: section.systemImage)
        .font(.system(size: 18, weight: .medium))
        .foregroundStyle(
          Color(
            uiColor: isSelected
              ? appearance.accent : appearance.secondaryText
          )
        )
        .frame(maxWidth: .infinity)
        .frame(height: 36)
        .background(
          isSelected ? Color(uiColor: appearance.control) : Color.clear,
          in: Circle()
        )
    }
    .buttonStyle(.plain)
    .accessibilityLabel(section.title)
    .accessibilityAddTraits(isSelected ? .isSelected : [])
  }
}

struct NativeEmojiRemoteImage: View {
  let url: URL
  let fallbackColor: UIColor

  @State private var phase: Phase = .loading

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
        phase = .success(
          try await NativeEmojiRemoteImageLoader.shared.image(for: request)
        )
      } catch {
        if !Task.isCancelled { phase = .failure }
      }
    }
  }

  private var requestIdentity: String {
    url.absoluteString
  }
}

enum NativeEmojiRemoteImageError: Error {
  case invalidResponse
  case responseTooLarge
  case invalidImage
}

actor NativeEmojiRemoteImageLoader {
  typealias Downloader = (URLRequest) async throws -> UIImage

  static let shared = NativeEmojiRemoteImageLoader()
  static let defaultMaximumConcurrentDownloads = 4

  private static let maximumDownloadBytes = 10 * 1024 * 1024
  private static let maximumThumbnailPixels = 84
  private static let defaultCacheByteLimit = 8 * 1024 * 1024

  private struct Waiter {
    let id: UUID
    let continuation: CheckedContinuation<Void, Error>
  }

  private let maximumConcurrentDownloads: Int
  private let downloader: Downloader
  private let admissionAttemptForTesting: (() -> Void)?
  private let cache = NSCache<NSURLRequest, UIImage>()
  private var activeDownloadCount = 0
  private var waiters: [Waiter] = []

  init(
    maximumConcurrentDownloads: Int = defaultMaximumConcurrentDownloads,
    cacheByteLimit: Int = defaultCacheByteLimit,
    admissionAttemptForTesting: (() -> Void)? = nil,
    downloader: @escaping Downloader = NativeEmojiRemoteImageLoader.download
  ) {
    precondition(maximumConcurrentDownloads > 0)
    precondition(cacheByteLimit >= 0)
    self.maximumConcurrentDownloads = maximumConcurrentDownloads
    self.admissionAttemptForTesting = admissionAttemptForTesting
    self.downloader = downloader
    cache.totalCostLimit = cacheByteLimit
  }

  func image(for request: URLRequest) async throws -> UIImage {
    let cacheKey = request as NSURLRequest
    if let cached = cache.object(forKey: cacheKey) {
      return cached
    }

    recordAdmissionAttemptForTesting()
    try await acquireDownloadSlot()
    defer { releaseDownloadSlot() }

    try Task.checkCancellation()
    if let cached = cache.object(forKey: cacheKey) {
      return cached
    }

    let image = try await downloader(request)
    cache.setObject(image, forKey: cacheKey, cost: Self.cacheCost(for: image))
    return image
  }

  private func recordAdmissionAttemptForTesting() {
    admissionAttemptForTesting?()
  }

  private func acquireDownloadSlot() async throws {
    try Task.checkCancellation()
    guard activeDownloadCount >= maximumConcurrentDownloads else {
      activeDownloadCount += 1
      return
    }

    let waiterID = UUID()
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation {
        (continuation: CheckedContinuation<Void, Error>) in
        if Task.isCancelled {
          continuation.resume(throwing: CancellationError())
        } else {
          waiters.append(Waiter(id: waiterID, continuation: continuation))
        }
      }
    } onCancel: {
      Task { await self.cancelWaiter(id: waiterID) }
    }
  }

  private func cancelWaiter(id: UUID) {
    guard let index = waiters.firstIndex(where: { $0.id == id }) else { return }
    let waiter = waiters.remove(at: index)
    waiter.continuation.resume(throwing: CancellationError())
  }

  private func releaseDownloadSlot() {
    while !waiters.isEmpty {
      let waiter = waiters.removeFirst()
      waiter.continuation.resume()
      return
    }
    activeDownloadCount -= 1
  }

  private static func download(_ request: URLRequest) async throws -> UIImage {
    let (bytes, response) = try await URLSession.shared.bytes(for: request)
    guard
      let httpResponse = response as? HTTPURLResponse,
      (200..<300).contains(httpResponse.statusCode)
    else {
      throw NativeEmojiRemoteImageError.invalidResponse
    }
    if let contentLength = httpResponse.value(forHTTPHeaderField: "Content-Length"),
      let byteCount = Int(contentLength),
      byteCount > maximumDownloadBytes
    {
      throw NativeEmojiRemoteImageError.responseTooLarge
    }

    var data = Data()
    let expected = httpResponse.expectedContentLength
    if expected > 0 {
      data.reserveCapacity(Int(min(expected, Int64(maximumDownloadBytes))))
    }
    for try await byte in bytes {
      guard data.count < maximumDownloadBytes else {
        throw NativeEmojiRemoteImageError.responseTooLarge
      }
      data.append(byte)
    }
    try Task.checkCancellation()
    guard let image = thumbnail(from: data) else {
      throw NativeEmojiRemoteImageError.invalidImage
    }
    return image
  }

  private static func thumbnail(from data: Data) -> UIImage? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
      return nil
    }
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceThumbnailMaxPixelSize: maximumThumbnailPixels,
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

  private static func cacheCost(for image: UIImage) -> Int {
    guard let cgImage = image.cgImage else { return 0 }
    let (cost, overflow) = cgImage.bytesPerRow.multipliedReportingOverflow(
      by: cgImage.height
    )
    return overflow ? Int.max : cost
  }
}
