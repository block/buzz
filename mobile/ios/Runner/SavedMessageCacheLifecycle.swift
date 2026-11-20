import Flutter
import UIKit

@MainActor
final class SavedMessageCacheLifecycle {
  typealias Expiration = @Sendable () -> Void

  static let shared = SavedMessageCacheLifecycle(
    worker: workerState,
    begin: { UIApplication.shared.beginBackgroundTask(withName: "Saved message cache", expirationHandler: $0) },
    end: { UIApplication.shared.endBackgroundTask($0) }
  )
  nonisolated fileprivate static let workerState = SavedMessageCacheWorkerState()

  private let worker: SavedMessageCacheWorkerState
  private let begin: (@escaping Expiration) -> UIBackgroundTaskIdentifier
  private let end: (UIBackgroundTaskIdentifier) -> Void
  private let waitForClose: (DispatchSemaphore) -> Void
  private var channel: FlutterMethodChannel?
  private var active = false
  private var deferredForeground = false
  private var nextID: Int64 = 0
  private var assertion: (id: Int64, task: UIBackgroundTaskIdentifier)?

  init(
    worker: SavedMessageCacheWorkerState,
    begin: @escaping (@escaping Expiration) -> UIBackgroundTaskIdentifier,
    end: @escaping (UIBackgroundTaskIdentifier) -> Void,
    waitForClose: @escaping (DispatchSemaphore) -> Void = {
      _ = $0.wait(timeout: .now() + .milliseconds(50))
    }
  ) {
    self.worker = worker
    self.begin = begin
    self.end = end
    self.waitForClose = waitForClose
  }

  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "buzz/saved_message_cache", binaryMessenger: messenger)
    self.channel = channel
    channel.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "acquire":
        result(self?.acquire().map { [$0, Int64(unsafeBitCast(cacheWorkerControl, to: Int.self))] })
      case "release":
        if let id = call.arguments as? NSNumber {
          self?.release(id.int64Value)
        }
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  func didBecomeActive() {
    active = true
    deferredForeground = true
    notifyForegroundWhenIdle()
  }

  func willResignActive() {
    active = false
    if let id = assertion?.id, worker.retire(id, onlyIfIdle: true) {
      endAssertion(id)
    }
  }

  func didEnterBackground() {
    willResignActive()
    if let id = assertion?.id {
      worker.cancel(id)
    }
  }

  func acquire() -> Int64? {
    guard active, assertion == nil else { return nil }
    nextID += 1
    let id = nextID
    guard worker.reserve(id) else { return nil }
    assertion = (id, .invalid)
    let task = begin { [weak self] in
      // UIKit invokes expiration synchronously on the main thread.
      MainActor.assumeIsolated { self?.expire(id) }
    }
    // An assertion can be refused, including by immediate expiration.
    guard assertion?.id == id else {
      if task != .invalid { end(task) }
      return nil
    }
    guard task != .invalid else {
      release(id)
      return nil
    }
    assertion?.task = task
    return id
  }

  func release(_ id: Int64) {
    guard assertion?.id == id, worker.retire(id) else { return }
    endAssertion(id)
  }

  fileprivate func workerFinished(_ id: Int64) {
    endAssertion(id)
    notifyForegroundWhenIdle()
  }

  private func expire(_ id: Int64) {
    guard assertion?.id == id else { return }
    let finished = worker.cancel(id)
    // The worker acknowledges after close without involving Flutter or UIKit.
    // UIKit still requires the assertion to end if filesystem I/O stalls.
    if let finished { waitForClose(finished) }
    _ = worker.retire(id)
    endAssertion(id)
  }

  private func endAssertion(_ id: Int64) {
    guard let assertion, assertion.id == id else { return }
    self.assertion = nil
    if assertion.task != .invalid { end(assertion.task) }
    notifyForegroundWhenIdle()
  }

  private func notifyForegroundWhenIdle() {
    guard active, deferredForeground, assertion == nil, worker.isIdle else { return }
    deferredForeground = false
    channel?.invokeMethod("foreground", arguments: nil)
  }
}

/// Cancellation and handle lifetime are guarded together. SQLite permits
/// interrupt from another thread only while its connection remains open.
final class SavedMessageCacheWorkerState: @unchecked Sendable {
  typealias Interrupt = @convention(c) (UnsafeMutableRawPointer) -> Void

  private final class Lease {
    let id: Int64
    let finished = DispatchSemaphore(value: 0)
    var running = false
    var cancelled = false
    var retired = false
    var connection: (UnsafeMutableRawPointer, Interrupt)?

    init(id: Int64) { self.id = id }
  }

  private let lock = NSLock()
  private var lease: Lease?

  var isIdle: Bool { lock.withLock { lease == nil } }

  func reserve(_ id: Int64) -> Bool {
    lock.withLock {
      guard lease == nil else { return false }
      lease = Lease(id: id)
      return true
    }
  }

  func start(_ id: Int64) -> Bool {
    lock.withLock {
      guard let lease, lease.id == id, !lease.cancelled, !lease.running else { return false }
      lease.running = true
      return true
    }
  }

  func register(_ id: Int64, handle: UnsafeMutableRawPointer, interrupt: Interrupt) -> Bool {
    lock.withLock {
      guard let lease, lease.id == id, lease.running, !lease.cancelled, lease.connection == nil else {
        return false
      }
      lease.connection = (handle, interrupt)
      return true
    }
  }

  func isCancelled(_ id: Int64) -> Bool {
    lock.withLock { lease?.id != id || lease?.cancelled == true }
  }

  func detach(_ id: Int64) {
    lock.withLock {
      if lease?.id == id { lease?.connection = nil }
    }
  }

  // True tells the bridge to release a task whose caller requested release
  // before the worker finished.
  func finish(_ id: Int64) -> Bool {
    lock.withLock {
      guard let lease, lease.id == id, lease.running, lease.connection == nil else { return false }
      lease.running = false
      lease.cancelled = true
      lease.finished.signal()
      if lease.retired { self.lease = nil }
      return lease.retired
    }
  }

  @discardableResult
  func cancel(_ id: Int64) -> DispatchSemaphore? {
    lock.withLock {
      guard let lease, lease.id == id else { return nil }
      lease.cancelled = true
      if let (handle, interrupt) = lease.connection { interrupt(handle) }
      return lease.running ? lease.finished : nil
    }
  }

  func retire(_ id: Int64, onlyIfIdle: Bool = false) -> Bool {
    lock.withLock {
      guard let lease, lease.id == id, !onlyIfIdle || !lease.running else { return false }
      lease.cancelled = true
      lease.retired = true
      if lease.running { return false }
      self.lease = nil
      return true
    }
  }
}

// Passed as a C function pointer over the channel. The worker never needs to
// queue a Flutter message to register, cancel-check, or acknowledge close.
private let cacheWorkerControl: @convention(c) (Int64, Int32, Int, Int) -> Int32 = {
  id, operation, handleAddress, interruptAddress in
  let worker = SavedMessageCacheLifecycle.workerState
  switch operation {
  case 0: return worker.start(id) ? 1 : 0
  case 1:
    guard let handle = UnsafeMutableRawPointer(bitPattern: handleAddress), interruptAddress != 0 else { return 0 }
    let interrupt = unsafeBitCast(interruptAddress, to: SavedMessageCacheWorkerState.Interrupt.self)
    return worker.register(id, handle: handle, interrupt: interrupt) ? 1 : 0
  case 2: return worker.isCancelled(id) ? 1 : 0
  case 3: worker.detach(id)
  case 4:
    if worker.finish(id) {
      DispatchQueue.main.async { SavedMessageCacheLifecycle.shared.workerFinished(id) }
    }
  default: break
  }
  return 0
}
