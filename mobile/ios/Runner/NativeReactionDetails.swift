import UIKit
import ImageIO
import CoreText

/// Native reaction toggles above the complete list of reactors.
final class NativeReactionDetailsViewController: UIViewController, UITableViewDataSource {
  var onClose: (() -> Void)?
  var onAddReaction: (() -> Void)?
  private let canAddReaction: Bool
  private let addReactionIconCodePoint: Int
  var onToggle: ((String) -> Void)?
  private let sheetColor: UIColor
  private let foregroundColor: UIColor
  private let outlineColor: UIColor
  private var reactions: [[String: Any]]
  private var profiles: [String: [String: Any]]
  private let table = UITableView(frame: .zero, style: .plain)
  private let filters = UIStackView()
  private var filterHeight: NSLayoutConstraint?
  private var rows: [(String, [String: Any])] = []
  private var burstGlyphs: [UILabel] = []

  private func rebuildRows() {
    rows = reactions.flatMap { reaction in
        (reaction["users"] as? [String] ?? []).map { ($0, reaction) }
      }
  }

  init(data: [String: Any]) {
    func color(_ key: String, fallback: UIColor) -> UIColor {
      guard let value = data[key] as? NSNumber else { return fallback }
      let argb = value.uint32Value
      return UIColor(red: CGFloat((argb >> 16) & 255) / 255,
        green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255,
        alpha: CGFloat((argb >> 24) & 255) / 255)
    }
    sheetColor = color("sheetColor", fallback: .systemGroupedBackground)
    foregroundColor = color("foregroundColor", fallback: .label)
    canAddReaction = data["canAddReaction"] as? Bool == true
    addReactionIconCodePoint = data["addReactionIconCodePoint"] as? Int ?? 0
    outlineColor = color("outlineColor", fallback: .separator)
    reactions = data["reactions"] as? [[String: Any]] ?? []
    profiles = data["profiles"] as? [String: [String: Any]] ?? [:]
    super.init(nibName: nil, bundle: nil)
    rebuildRows()
    overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = sheetColor
    view.accessibilityViewIsModal = true
    let title = UILabel()
    title.text = "Reactions"
    title.font = .preferredFont(forTextStyle: .headline)
    title.adjustsFontForContentSizeCategory = true
    title.textAlignment = .center
    title.textColor = foregroundColor
    title.accessibilityTraits = .header
    let close = NativeReactionCloseButton(primaryAction: UIAction { [weak self] _ in
      UIImpactFeedbackGenerator(style: .light).impactOccurred()
      self?.onClose?()
    })
    var configuration: UIButton.Configuration
    if #available(iOS 26.0, *) {
      configuration = .glass()
    } else {
      configuration = .gray()
      configuration.baseBackgroundColor = .secondarySystemBackground
    }
    configuration.cornerStyle = .capsule
    configuration.image = UIImage(systemName: "xmark",
      withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold))
    configuration.baseForegroundColor = foregroundColor
    close.configuration = configuration
    close.tintColor = foregroundColor
    close.accessibilityLabel = "Close reactions"
    let header = UIView()
    for child in [title, close] {
      child.translatesAutoresizingMaskIntoConstraints = false
      header.addSubview(child)
    }
    NSLayoutConstraint.activate([
      header.heightAnchor.constraint(equalToConstant: max(56, title.font.lineHeight + 16)),
      title.centerXAnchor.constraint(equalTo: header.centerXAnchor),
      title.centerYAnchor.constraint(equalTo: close.centerYAnchor),
      title.leadingAnchor.constraint(greaterThanOrEqualTo: header.leadingAnchor, constant: 64),
      title.trailingAnchor.constraint(lessThanOrEqualTo: header.trailingAnchor, constant: -64),
      title.topAnchor.constraint(greaterThanOrEqualTo: header.topAnchor, constant: 4),
      close.widthAnchor.constraint(equalToConstant: 40),
      close.heightAnchor.constraint(equalToConstant: 40),
      close.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -2),
      close.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -2),
    ])
    let filterScroll = UIScrollView()
    filterScroll.showsHorizontalScrollIndicator = false
    filters.spacing = 8
    filters.translatesAutoresizingMaskIntoConstraints = false
    filterScroll.addSubview(filters)
    NSLayoutConstraint.activate([
      filters.leadingAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.leadingAnchor),
      filters.trailingAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.trailingAnchor),
      filters.topAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.topAnchor),
      filters.bottomAnchor.constraint(equalTo: filterScroll.contentLayoutGuide.bottomAnchor),
      filters.heightAnchor.constraint(equalTo: filterScroll.frameLayoutGuide.heightAnchor),
    ])
    rebuildFilters()
    filterHeight = filterScroll.heightAnchor.constraint(equalToConstant: 48)
    filterHeight?.isActive = true
    table.dataSource = self
    table.rowHeight = UITableView.automaticDimension
    table.estimatedRowHeight = 64
    table.backgroundColor = .clear
    table.separatorInset = UIEdgeInsets(top: 0, left: 72, bottom: 0, right: 16)
    let layout = UIStackView(arrangedSubviews: [header, filterScroll, table])
    layout.axis = .vertical
    layout.spacing = 12
    layout.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(layout)
    NSLayoutConstraint.activate([
      layout.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 16),
      layout.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
      layout.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 4),
      layout.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
    ])
  }

  override func viewWillLayoutSubviews() {
    super.viewWillLayoutSubviews()
    // Horizontal scrolling must not constrain Dynamic Type button height.
    filterHeight?.constant = filters.arrangedSubviews.reduce(CGFloat(48)) { height, button in
      max(height, button.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize).height)
    }
  }

  // Use the exact bundled Tabler font that backs BuzzIcons in Flutter.
  private static let tablerFont: UIFont? = {
    guard let frameworks = Bundle.main.privateFrameworksURL,
      let provider = CGDataProvider(url: frameworks.appendingPathComponent(
        "App.framework/flutter_assets/assets/fonts/TablerIcons.ttf") as CFURL),
      let font = CGFont(provider), let name = font.postScriptName else { return nil }
    CTFontManagerRegisterGraphicsFont(font, nil)
    return UIFont(name: name as String, size: 24)
  }()

  private var addReactionImage: UIImage? {
    guard let font = Self.tablerFont,
      let scalar = UnicodeScalar(addReactionIconCodePoint) else { return nil }
    let glyph = NSAttributedString(string: String(scalar), attributes: [.font: font])
    let size = glyph.size()
    return UIGraphicsImageRenderer(size: CGSize(width: 28, height: 28)).image { _ in
      glyph.draw(at: CGPoint(x: (28 - size.width) / 2, y: (28 - size.height) / 2))
    }.withRenderingMode(.alwaysTemplate)
  }

  private func rebuildFilters() {
    for child in filters.arrangedSubviews { child.removeFromSuperview() }
    if canAddReaction {
      let add = UIButton(type: .system)
      var config = UIButton.Configuration.gray()
      config.image = addReactionImage
      if config.image == nil { config.title = "Add reaction" }
      config.cornerStyle = .capsule
      config.baseForegroundColor = foregroundColor
      add.configuration = config
      add.accessibilityLabel = "Add reaction"
      add.accessibilityIdentifier = "reaction-sheet-add"
      add.widthAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
      add.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
      add.addAction(UIAction { [weak self] _ in self?.onAddReaction?() }, for: .touchUpInside)
      filters.addArrangedSubview(add)
    }
    for reaction in reactions {
      guard let emoji = reaction["emoji"] as? String else { continue }
      addFilter(title: "\(reaction["count"] as? Int ?? 0)", emoji: emoji, reaction: reaction)
    }
  }

  private func addFilter(title: String, emoji: String?, reaction: [String: Any]?) {
    var config = UIButton.Configuration.tinted()
    config.cornerStyle = .capsule
    config.title = reaction == nil ? title : "       \(title)"
    let selected = reaction?["selected"] as? Bool == true
    config.baseBackgroundColor = selected ? foregroundColor : .tertiarySystemFill
    config.baseForegroundColor = selected ? foregroundColor : .label
    config.background.strokeWidth = 1
    let stroke = selected ? foregroundColor.withAlphaComponent(0.40) : outlineColor
    config.background.strokeColor = stroke
    config.background.strokeColorTransformer = UIConfigurationColorTransformer { _ in stroke }
    let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] action in
      guard let self else { return }
      guard let emoji else { return }
      if !selected, reaction?["burst"] as? Bool == true,
        let button = action.sender as? UIButton {
        self.burst(emoji, from: button)
      }
      self.onToggle?(emoji)
      UISelectionFeedbackGenerator().selectionChanged()
    })
    button.accessibilityLabel = reaction.map { "\($0["label"] as? String ?? emoji ?? "") \(title)" } ?? title
    if selected { button.accessibilityTraits.insert(.selected) }
    if let reaction {
      let glyph = NativeMessageGlyph(data: reaction, size: 22)
      glyph.isUserInteractionEnabled = false
      glyph.translatesAutoresizingMaskIntoConstraints = false
      button.addSubview(glyph)
      NSLayoutConstraint.activate([
        glyph.leadingAnchor.constraint(equalTo: button.leadingAnchor, constant: 12),
        glyph.centerYAnchor.constraint(equalTo: button.centerYAnchor),
        glyph.widthAnchor.constraint(equalToConstant: 26),
        glyph.heightAnchor.constraint(equalToConstant: 26),
      ])
    }
    filters.addArrangedSubview(button)
  }

  /// Mirrors Flutter EmojiBurstController's five-particle, fixed-60Hz physics.
  /// UIKit owns this sheet, so Flutter's underlying overlay cannot paint above it.
  private func burst(_ emoji: String, from button: UIView) {
    guard !UIAccessibility.isReduceMotionEnabled,
      burstGlyphs.count + 5 <= 160, let window = view.window else { return }
    let origin = button.convert(CGPoint(x: button.bounds.midX, y: button.bounds.midY), to: window)
    for _ in 0..<5 {
      let glyph = UILabel()
      glyph.text = emoji
      glyph.font = .systemFont(ofSize: 18 + ceil(CGFloat.random(in: 0..<24)))
      glyph.sizeToFit()
      glyph.center = origin
      glyph.isUserInteractionEnabled = false
      glyph.isAccessibilityElement = false
      window.addSubview(glyph)
      burstGlyphs.append(glyph)
      var x = origin.x, y = origin.y
      var xv = CGFloat.random(in: -2.2..<2.2)
      var yv = -CGFloat.random(in: 2.1..<4.45)
      var rotation = CGFloat.random(in: -11..<11)
      let spin = CGFloat.random(in: -2.6..<2.6)
      let gravity = -CGFloat.random(in: 0.018..<0.036)
      var scale: CGFloat = 0.25
      var positions = [NSValue(cgPoint: origin)]
      var rotations = [rotation * .pi / 180]
      var scales = [scale]
      var opacities: [CGFloat] = [1]
      for frame in 1...108 {
        rotation += spin
        yv = (yv + gravity) * 0.998
        xv *= 0.965
        x += xv
        y += yv
        scale += (1 - scale) * 0.28
        positions.append(NSValue(cgPoint: CGPoint(x: x, y: y)))
        rotations.append(rotation * .pi / 180)
        scales.append(scale)
        opacities.append(min(1, CGFloat(108 - frame) / (108 * 0.24)))
      }
      let values: [(String, [Any])] = [
        ("position", positions), ("transform.rotation.z", rotations),
        ("transform.scale", scales), ("opacity", opacities),
      ]
      let group = CAAnimationGroup()
      group.duration = 108.0 / 60.0
      group.animations = values.map { key, frames in
        let animation = CAKeyframeAnimation(keyPath: key)
        animation.values = frames
        animation.duration = group.duration
        animation.calculationMode = .linear
        return animation
      }
      glyph.layer.opacity = 0
      glyph.layer.add(group, forKey: "reaction-burst")
      DispatchQueue.main.asyncAfter(deadline: .now() + group.duration) { [weak self, weak glyph] in
        glyph?.removeFromSuperview()
        self?.burstGlyphs.removeAll { $0 === glyph }
      }
    }
  }

  override func viewDidDisappear(_ animated: Bool) {
    super.viewDidDisappear(animated)
    burstGlyphs.forEach { $0.removeFromSuperview() }
    burstGlyphs.removeAll()
  }

  func updateReactions(_ reactions: [[String: Any]]) {
    guard !reactions.isEmpty else { onClose?(); return }
    self.reactions = reactions
    rebuildRows()
    guard isViewLoaded else { return }
    rebuildFilters()
    table.reloadData()
  }

  func updateProfiles(_ profiles: [String: [String: Any]]) {
    self.profiles.merge(profiles) { _, new in new }
    guard isViewLoaded else { return }
    let affected = (table.indexPathsForVisibleRows ?? []).filter {
      profiles[rows[$0.row].0] != nil
    }
    if !affected.isEmpty { table.reloadRows(at: affected, with: .none) }
  }

  func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { rows.count }

  func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    let (pubkey, reaction) = rows[indexPath.row]
    let profile = profiles[pubkey] ?? [:]
    let name = profile["name"] as? String ?? String(pubkey.prefix(8))
    let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
    cell.backgroundColor = .clear
    cell.selectionStyle = .none
    let avatar = NativeMessageGlyph(data: profile.merging(["emoji": profile["avatarEmoji"] as? String ?? String(name.prefix(1))]) { _, new in new }, size: 18)
    if let color = profile["avatarColor"] as? NSNumber {
      let argb = color.uint32Value
      avatar.backgroundColor = UIColor(red: CGFloat((argb >> 16) & 255) / 255,
        green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255,
        alpha: CGFloat((argb >> 24) & 255) / 255)
    } else {
      avatar.backgroundColor = .tertiarySystemFill
    }
    avatar.layer.cornerRadius = 20
    avatar.clipsToBounds = true
    let label = UILabel()
    label.text = name
    label.font = .preferredFont(forTextStyle: .body)
    label.adjustsFontForContentSizeCategory = true
    label.numberOfLines = 0
    let emoji = NativeMessageGlyph(data: reaction, size: 26)
    let row = UIStackView(arrangedSubviews: [avatar, label, emoji])
    row.alignment = .center
    row.spacing = 12
    row.translatesAutoresizingMaskIntoConstraints = false
    cell.contentView.addSubview(row)
    NSLayoutConstraint.activate([
      avatar.widthAnchor.constraint(equalToConstant: 40), avatar.heightAnchor.constraint(equalToConstant: 40),
      emoji.widthAnchor.constraint(equalToConstant: 32), emoji.heightAnchor.constraint(equalToConstant: 32),
      row.leadingAnchor.constraint(equalTo: cell.contentView.leadingAnchor),
      row.trailingAnchor.constraint(equalTo: cell.contentView.trailingAnchor),
      row.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 10),
      row.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -10),
    ])
    cell.isAccessibilityElement = true
    cell.accessibilityLabel = "\(name), \(reaction["label"] as? String ?? reaction["emoji"] as? String ?? "reaction")"
    return cell
  }

  override func accessibilityPerformEscape() -> Bool { onClose?(); return true }
}

/// Match the profile sheet's 40-point glass control and 44-point hit target.
private final class NativeReactionCloseButton: UIButton {
  override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
    bounds.insetBy(dx: -2, dy: -2).contains(point)
  }
}

/// Short-lived image loader. Auth is supplied for the specific URL by Buzz's
/// existing media auth service; redirects are refused to avoid forwarding it.
final class NativeMessageGlyph: UIView {
  private var cancelLoad: (() -> Void)?
  private let label = UILabel()
  private let image = UIImageView()

  init(data: [String: Any], size: CGFloat) {
    super.init(frame: .zero)
    label.text = data["emoji"] as? String
    label.textAlignment = .center
    label.font = .systemFont(ofSize: size)
    label.adjustsFontSizeToFitWidth = true
    label.minimumScaleFactor = 0.4
    image.contentMode = .scaleAspectFit
    for child in [label, image] {
      child.translatesAutoresizingMaskIntoConstraints = false
      addSubview(child)
      NSLayoutConstraint.activate([
        child.leadingAnchor.constraint(equalTo: leadingAnchor),
        child.trailingAnchor.constraint(equalTo: trailingAnchor),
        child.topAnchor.constraint(equalTo: topAnchor),
        child.bottomAnchor.constraint(equalTo: bottomAnchor),
      ])
    }
    guard let raw = data["url"] as? String, let url = URL(string: raw),
      ["https", "http"].contains(url.scheme?.lowercased() ?? "") else { return }
    var request = URLRequest(url: url)
    request.allHTTPHeaderFields = data["headers"] as? [String: String]
    request.timeoutInterval = 10
    cancelLoad = NativeMessageImageLoader.shared.load(request) { [weak self] loaded in
      guard let self, let loaded else { return }
      image.image = UIAccessibility.isReduceMotionEnabled ? loaded.images?.first ?? loaded : loaded
      label.isHidden = true
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  deinit { cancelLoad?() }
}

/// Shared, main-queue-owned loader. Requests include auth headers in their cache
/// key; duplicate consumers share one download and cancellation releases their
/// subscription. No more than four bodies are admitted, each capped at 8 MiB.
final class NativeMessageImageLoader: NSObject, URLSessionDataDelegate {
  static let shared = NativeMessageImageLoader()
  static let maximumBytes = 8 * 1024 * 1024
  private final class Download {
    let request: URLRequest
    var callbacks: [UUID: (UIImage?) -> Void] = [:]
    var task: URLSessionDataTask?
    var bytes = Data()
    init(_ request: URLRequest) { self.request = request }
  }
  private let configuration: URLSessionConfiguration
  private let limit: Int
  private lazy var session = URLSession(configuration: configuration, delegate: self, delegateQueue: .main)
  private var downloads: [URLRequest: Download] = [:]
  private var active: [Int: Download] = [:]
  private var queue: [Download] = []
  private let cache = NSCache<NSURLRequest, UIImage>()

  init(configuration: URLSessionConfiguration = .ephemeral, maximumConcurrent: Int = 4) {
    precondition(maximumConcurrent > 0)
    self.configuration = configuration
    self.limit = maximumConcurrent
    super.init()
    cache.totalCostLimit = 16 * 1024 * 1024
  }

  /// Call on the main queue; the returned cancellation is safe from any queue.
  func load(_ request: URLRequest, completion: @escaping (UIImage?) -> Void) -> () -> Void {
    dispatchPrecondition(condition: .onQueue(.main))
    if let image = cache.object(forKey: request as NSURLRequest) {
      completion(image)
      return {}
    }
    let id = UUID()
    let download = downloads[request] ?? Download(request)
    download.callbacks[id] = completion
    if downloads[request] == nil {
      downloads[request] = download
      queue.append(download)
      admit()
    }
    return { [weak self, weak download] in
      DispatchQueue.main.async {
        guard let self, let download else { return }
        download.callbacks.removeValue(forKey: id)
        if download.callbacks.isEmpty {
          download.task?.cancel()
          self.finish(download, image: nil)
        }
      }
    }
  }

  private func admit() {
    while active.count < limit, !queue.isEmpty {
      let download = queue.removeFirst()
      let task = session.dataTask(with: download.request)
      download.task = task
      active[task.taskIdentifier] = download
      task.resume()
    }
  }

  private func finish(_ download: Download, image: UIImage?) {
    guard downloads[download.request] === download else { return }
    downloads.removeValue(forKey: download.request)
    queue.removeAll { $0 === download }
    if let task = download.task { active.removeValue(forKey: task.taskIdentifier) }
    if let image {
      let frames = image.images ?? [image]
      let cost = frames.reduce(0) { $0 + ($1.cgImage.map { $0.bytesPerRow * $0.height } ?? 0) }
      cache.setObject(image, forKey: download.request as NSURLRequest, cost: cost)
    }
    let callbacks = Array(download.callbacks.values)
    download.callbacks.removeAll()
    admit()
    for callback in callbacks { callback(image) }
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask,
    didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
    guard let download = active[dataTask.taskIdentifier] else { completionHandler(.cancel); return }
    let declaredLength = (response as? HTTPURLResponse)?
      .value(forHTTPHeaderField: "Content-Length").flatMap(Int64.init) ?? -1
    guard let http = response as? HTTPURLResponse, http.statusCode == 200,
      declaredLength <= Int64(Self.maximumBytes),
      response.expectedContentLength <= Int64(Self.maximumBytes) else {
      completionHandler(.cancel)
      finish(download, image: nil)
      return
    }
    completionHandler(.allow)
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    guard let download = active[dataTask.taskIdentifier] else { return }
    guard data.count <= Self.maximumBytes - download.bytes.count else {
      dataTask.cancel()
      finish(download, image: nil)
      return
    }
    download.bytes.append(data)
  }

  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
    guard let download = active[task.taskIdentifier] else { return }
    guard error == nil else { finish(download, image: nil); return }
    let bytes = download.bytes
    download.bytes = Data()
    // Keep the admission slot through decoding, bounding CPU and decoded images
    // as well as network bodies. Cancellation still suppresses delivery/cache.
    DispatchQueue.global(qos: .userInitiated).async { [weak self, weak download] in
      let image = Self.decode(bytes)
      DispatchQueue.main.async {
        guard let self, let download else { return }
        self.finish(download, image: image)
      }
    }
  }

  func urlSession(_ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
    completionHandler: @escaping (URLRequest?) -> Void) {
    completionHandler(nil) // Never forward supplied auth to another URL.
  }

  private static func decode(_ bytes: Data) -> UIImage? {
    guard let source = CGImageSourceCreateWithData(bytes as CFData, nil) else { return nil }
    let count = CGImageSourceGetCount(source)
    let step = max(1, Int(ceil(Double(count) / 60)))
    var frames: [UIImage] = []
    var duration: TimeInterval = 0
    for index in stride(from: 0, to: count, by: step) {
      guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceThumbnailMaxPixelSize: 120,
        kCGImageSourceCreateThumbnailWithTransform: true,
      ] as CFDictionary) else { continue }
      frames.append(UIImage(cgImage: image))
      let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [String: Any]
      let animation = (properties?[kCGImagePropertyGIFDictionary as String]
        ?? properties?[kCGImagePropertyPNGDictionary as String] ?? properties?["{WebP}"]) as? [String: Any]
      let delay = (animation?["UnclampedDelayTime"] ?? animation?["DelayTime"]) as? Double ?? 0.1
      duration += max(0.02, delay) * Double(step)
    }
    guard let first = frames.first else { return nil }
    return frames.count > 1 ? UIImage.animatedImage(with: frames, duration: duration) : first
  }
}
