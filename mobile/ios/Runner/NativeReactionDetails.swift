import UIKit
import ImageIO

/// Native resizable sheet: selected emoji first, with All and per-emoji filters.
final class NativeReactionDetailsViewController: UIViewController, UITableViewDataSource {
  var onClose: (() -> Void)?
  private let sheetColor: UIColor
  private let foregroundColor: UIColor
  private let reactions: [[String: Any]]
  private var profiles: [String: [String: Any]]
  private var selectedEmoji: String?
  private let table = UITableView(frame: .zero, style: .plain)
  private let filters = UIStackView()
  private var filterHeight: NSLayoutConstraint?
  private var rows: [(String, [String: Any])] = []

  private func rebuildRows() {
    rows = reactions.filter { selectedEmoji == nil || $0["emoji"] as? String == selectedEmoji }
      .flatMap { reaction in
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
    reactions = data["reactions"] as? [[String: Any]] ?? []
    profiles = data["profiles"] as? [String: [String: Any]] ?? [:]
    let initial = data["initialEmoji"] as? String
    selectedEmoji = reactions.contains { $0["emoji"] as? String == initial } ? initial : nil
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

  private func rebuildFilters() {
    for child in filters.arrangedSubviews { child.removeFromSuperview() }
    let count = reactions.reduce(0) { $0 + ($1["count"] as? Int ?? 0) }
    addFilter(title: "All \(count)", emoji: nil, reaction: nil)
    for reaction in reactions {
      guard let emoji = reaction["emoji"] as? String else { continue }
      addFilter(title: "\(reaction["count"] as? Int ?? 0)", emoji: emoji, reaction: reaction)
    }
  }

  private func addFilter(title: String, emoji: String?, reaction: [String: Any]?) {
    var config = UIButton.Configuration.tinted()
    config.cornerStyle = .capsule
    config.title = reaction == nil ? title : "       \(title)"
    config.baseBackgroundColor = selectedEmoji == emoji ? .systemBlue : .tertiarySystemFill
    config.baseForegroundColor = selectedEmoji == emoji ? .systemBlue : .label
    let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
      guard let self else { return }
      guard self.selectedEmoji != emoji else { return }
      self.selectedEmoji = emoji
      self.rebuildRows()
      self.rebuildFilters()
      self.table.reloadData()
      self.table.setContentOffset(.zero, animated: false)
      UISelectionFeedbackGenerator().selectionChanged()
    })
    button.accessibilityLabel = reaction.map { "\($0["label"] as? String ?? emoji ?? "") \(title)" } ?? title
    if selectedEmoji == emoji { button.accessibilityTraits.insert(.selected) }
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
