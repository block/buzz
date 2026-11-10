import Foundation

/// A bounded ordinary URLSession transfer. Completion does not require a background session.
final class BuzzNotificationDownload: NSObject, URLSessionDataDelegate, @unchecked Sendable {
  enum Failure: Error {
    case tooLarge
    case status(Int)
    /// A 401 carrying `WWW-Authenticate: Nostr`: the relay requires NIP-FI evidence.
    case challenged
    case empty, invalidResponse
  }
  private let maximumBytes: Int
  private let completion: (Result<Data, Error>) -> Void
  private var received = Data()
  private var failure: Error?
  private var session: URLSession?

  private init(maximumBytes: Int, completion: @escaping (Result<Data, Error>) -> Void) {
    self.maximumBytes = maximumBytes
    self.completion = completion
  }

  static func start(
    request: URLRequest, configuration: URLSessionConfiguration,
    maximumBytes: Int, completion: @escaping (Result<Data, Error>) -> Void
  ) {
    let transfer = BuzzNotificationDownload(maximumBytes: maximumBytes, completion: completion)
    let session = URLSession(configuration: configuration, delegate: transfer, delegateQueue: nil)
    transfer.session = session
    session.dataTask(with: request).resume()
  }

  func urlSession(
    _ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
  ) {
    guard let response = response as? HTTPURLResponse else {
      failure = Failure.invalidResponse
      completionHandler(.cancel)
      return
    }
    guard (200..<300).contains(response.statusCode) else {
      failure =
        BuzzRelayAssertionIssuer.isChallenge(response)
        ? Failure.challenged : Failure.status(response.statusCode)
      completionHandler(.cancel)
      return
    }
    guard response.expectedContentLength <= Int64(maximumBytes) else {
      failure = Failure.tooLarge
      completionHandler(.cancel)
      return
    }
    completionHandler(.allow)
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    guard received.count + data.count <= maximumBytes else {
      failure = Failure.tooLarge
      dataTask.cancel()
      return
    }
    received.append(data)
  }

  func urlSession(
    _ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?
  ) {
    defer {
      session.finishTasksAndInvalidate()
      self.session = nil
    }
    if let error = failure ?? error {
      completion(.failure(error))
    } else if received.isEmpty {
      completion(.failure(Failure.empty))
    } else {
      completion(.success(received))
    }
  }

  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
    completionHandler: @escaping (URLRequest?) -> Void
  ) {
    // Signed relay requests and account requests must not follow redirects or
    // forward their proof or session.
    let original = task.originalRequest
    guard
      ["Authorization", "X-BB-Session-Credential", "Nostr-Federated-Identity"].allSatisfy({
        original?.value(forHTTPHeaderField: $0) == nil
      }),
      let url = request.url, ["https", "http"].contains(url.scheme?.lowercased() ?? ""),
      url.user == nil, url.password == nil,
      !(task.originalRequest?.url?.scheme == "https" && url.scheme != "https")
    else {
      completionHandler(nil)
      return
    }
    var redirected = request
    redirected.setValue(nil, forHTTPHeaderField: "Authorization")
    completionHandler(redirected)
  }
}
