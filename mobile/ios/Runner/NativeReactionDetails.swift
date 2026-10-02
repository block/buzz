import UIKit
import ImageIO

/// Native resizable sheet: selected emoji first, with All and per-emoji filters.
final class NativeReactionDetailsViewController: UIViewController, UITableViewDataSource {
  var onClose: (() -> Void)?
  private let reactions: [[String: Any]]
  private var profiles: [String: [String: Any]]
  private var selectedEmoji: String?
  private let table = UITableView(frame: .zero, style: .plain)
  private let filters = UIStackView()
  private var rows: [(String, [String: Any])] {
    reactions.filter { selectedEmoji == nil || $0["emoji"] as? String == selectedEmoji }
      .flatMap { reaction in
        (reaction["users"] as? [String] ?? []).map { ($0, reaction) }
      }
  }

  init(data: [String: Any]) {
    reactions = data["reactions"] as? [[String: Any]] ?? []
    profiles = data["profiles"] as? [String: [String: Any]] ?? [:]
    let initial = data["initialEmoji"] as? String
    selectedEmoji = reactions.contains { $0["emoji"] as? String == initial } ? initial : nil
    super.init(nibName: nil, bundle: nil)
    overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground
    view.accessibilityViewIsModal = true
    let title = UILabel()
    title.text = "Reactions"
    title.font = .preferredFont(forTextStyle: .headline)
    title.adjustsFontForContentSizeCategory = true
    title.accessibilityTraits = .header
    let close = UIButton(primaryAction: UIAction { [weak self] _ in self?.onClose?() })
    close.setImage(UIImage(systemName: "xmark.circle.fill"), for: .normal)
    close.tintColor = .secondaryLabel
    close.accessibilityLabel = "Close reactions"
    close.widthAnchor.constraint(equalToConstant: 44).isActive = true
    close.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
    let header = UIStackView(arrangedSubviews: [title, close])
    header.spacing = 16
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
      layout.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 20),
      layout.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
      filterScroll.heightAnchor.constraint(equalToConstant: 48),
    ])
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
      self.selectedEmoji = emoji
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
    self.profiles = profiles
    if isViewLoaded { table.reloadData() }
  }

  func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { rows.count }

  func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    let (pubkey, reaction) = rows[indexPath.row]
    let profile = profiles[pubkey] ?? [:]
    let name = profile["name"] as? String ?? String(pubkey.prefix(8))
    let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
    cell.selectionStyle = .none
    let avatar = NativeMessageGlyph(data: profile.merging(["emoji": String(name.prefix(1))]) { _, new in new }, size: 18)
    avatar.backgroundColor = .tertiarySystemFill
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

/// Short-lived image loader. Auth is supplied for the specific URL by Buzz's
/// existing media auth service; redirects are refused to avoid forwarding it.
final class NativeMessageGlyph: UIView, URLSessionTaskDelegate {
  private var task: URLSessionDataTask?
  private var session: URLSession?
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
    let session = URLSession(configuration: .ephemeral, delegate: self, delegateQueue: nil)
    self.session = session
    task = session.dataTask(with: request) { [weak self, weak session] bytes, response, _ in
      defer { session?.finishTasksAndInvalidate() }
      guard let bytes, bytes.count <= 8 * 1024 * 1024,
        (response as? HTTPURLResponse)?.statusCode == 200,
        let source = CGImageSourceCreateWithData(bytes as CFData, nil) else { return }
      let frameCount = CGImageSourceGetCount(source)
      let stride = max(1, Int(ceil(Double(frameCount) / 60)))
      var frames: [UIImage] = []
      var duration: TimeInterval = 0
      for index in Swift.stride(from: 0, to: frameCount, by: stride) {
        guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, index, [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceThumbnailMaxPixelSize: 120,
          kCGImageSourceCreateThumbnailWithTransform: true,
        ] as CFDictionary) else { continue }
        frames.append(UIImage(cgImage: thumbnail))
        let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [String: Any]
        let animation = (properties?[kCGImagePropertyGIFDictionary as String]
          ?? properties?[kCGImagePropertyPNGDictionary as String]
          ?? properties?["{WebP}"]) as? [String: Any]
        let delay = (animation?["UnclampedDelayTime"] ?? animation?["DelayTime"]) as? Double ?? 0.1
        duration += max(0.02, delay) * Double(stride)
      }
      DispatchQueue.main.async {
        guard let first = frames.first else { return }
        self?.image.image = frames.count > 1 && !UIAccessibility.isReduceMotionEnabled
          ? UIImage.animatedImage(with: frames, duration: duration) : first
        self?.label.isHidden = true
      }
    }
    task?.resume()
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  func urlSession(_ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
    completionHandler: @escaping (URLRequest?) -> Void) {
    completionHandler(nil)
  }

  deinit { task?.cancel(); session?.invalidateAndCancel() }
}
