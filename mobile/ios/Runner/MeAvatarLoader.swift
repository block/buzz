import UIKit
import ImageIO

/// Visible-row avatar loading with bounded network, decoded memory, and lifetime.
/// All entry points and URLSession callbacks run on the main queue.
final class MeAvatarLoader: NSObject, URLSessionDataDelegate, @unchecked Sendable {
  static let byteLimit = 2 * 1024 * 1024
  static let pixelLimit = 16_000_000
  static let concurrency = 3
  private let configuration: URLSessionConfiguration
  private var session: URLSession?
  private let cache = NSCache<NSURL, UIImage>()
  private struct Request {
    let url: URL
    let completion: (UIImage?) -> Void
    var task: URLSessionDataTask?
    var data = Data()
  }
  private var requests: [UUID: Request] = [:]
  private var queue: [UUID] = []

  init(configuration: URLSessionConfiguration = .ephemeral) {
    self.configuration = configuration
    super.init()
    cache.totalCostLimit = 4 * 1024 * 1024
    cache.countLimit = 128
  }

  func image(for url: URL?) -> UIImage? {
    guard let url else { return nil }
    return cache.object(forKey: url as NSURL)
  }

  @discardableResult
  func load(_ url: URL, completion: @escaping (UIImage?) -> Void) -> UUID {
    let id = UUID()
    guard ["http", "https"].contains(url.scheme?.lowercased() ?? "") else {
      completion(nil)
      return id
    }
    if let cached = image(for: url) {
      completion(cached)
      return id
    }
    requests[id] = Request(url: url, completion: completion)
    queue.append(id)
    startQueued()
    return id
  }

  func cancel(_ id: UUID) {
    requests.removeValue(forKey: id)?.task?.cancel()
    queue.removeAll { $0 == id }
    startQueued()
  }

  func cancelAll() {
    queue.removeAll()
    requests.removeAll()
    session?.invalidateAndCancel()
    session = nil
  }

  private func startQueued() {
    while requests.values.filter({ $0.task != nil }).count < Self.concurrency,
          !queue.isEmpty {
      let id = queue.removeFirst()
      guard let request = requests[id] else { continue }
      if session == nil {
        configuration.urlCache = nil
        configuration.timeoutIntervalForRequest = 15
        session = URLSession(configuration: configuration, delegate: self, delegateQueue: .main)
      }
      let task = session!.dataTask(with: request.url)
      requests[id]?.task = task
      task.resume()
    }
  }

  private func id(for task: URLSessionTask) -> UUID? {
    requests.first { $0.value.task === task }?.key
  }

  private func finish(_ id: UUID, image: UIImage?) {
    guard let request = requests.removeValue(forKey: id) else { return }
    if let image, let cgImage = image.cgImage {
      cache.setObject(image, forKey: request.url as NSURL, cost: cgImage.bytesPerRow * cgImage.height)
    }
    request.completion(image)
    startQueued()
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask,
                  didReceive response: URLResponse,
                  completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
    guard let id = id(for: dataTask) else { completionHandler(.cancel); return }
    guard let response = response as? HTTPURLResponse,
          (200...299).contains(response.statusCode),
          response.mimeType?.hasPrefix("image/") == true,
          response.expectedContentLength <= Self.byteLimit else {
      completionHandler(.cancel)
      finish(id, image: nil)
      return
    }
    completionHandler(.allow)
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    guard let id = id(for: dataTask), let request = requests[id] else { return }
    guard data.count <= Self.byteLimit - request.data.count else {
      dataTask.cancel()
      finish(id, image: nil)
      return
    }
    requests[id]?.data.append(data)
  }

  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
    guard let id = id(for: task), let request = requests[id] else { return }
    finish(id, image: error == nil ? Self.thumbnail(request.data) : nil)
  }

  static func thumbnail(_ data: Data) -> UIImage? {
    guard let source = CGImageSourceCreateWithData(data as CFData,
            [kCGImageSourceShouldCache: false] as CFDictionary),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0, width <= pixelLimit / height,
          let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 120,
            kCGImageSourceShouldCacheImmediately: true,
          ] as CFDictionary) else { return nil }
    return UIImage(cgImage: image)
  }
}
