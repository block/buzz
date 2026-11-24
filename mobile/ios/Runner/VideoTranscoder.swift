import AVFoundation
import UIKit

/// Prepares a video for upload as Buzz accepts it, and makes its poster
/// frame. Shared by the app and its share extension.
enum VideoTranscoder {
  struct Failure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
  }

  /// Re-encodes the video at `source` to a canonical H.264/AAC MP4 with no
  /// metadata, in the temporary directory, and calls back with it.
  static func transcode(source sourceURL: URL, completion: @escaping (Result<URL, Error>) -> Void) {
    let asset = AVURLAsset(url: sourceURL)

    // Do not export the source asset directly. An iPhone video can carry GPS,
    // spatial-video, and other data tracks even when its user-visible metadata
    // is cleared. A fresh composition copies only one video and one audio
    // track, so those private channels cannot reach the relay.
    let composition = AVMutableComposition()
    guard
      let sourceVideo = asset.tracks(withMediaType: .video).first,
      let destinationVideo = composition.addMutableTrack(
        withMediaType: .video,
        preferredTrackID: kCMPersistentTrackID_Invalid
      )
    else {
      return completion(.failure(Failure(message: "The selected file does not contain a video track.")))
    }

    do {
      let sourceAudio = asset.tracks(withMediaType: .audio).first
      let insertionTimes = relativeTrackInsertionTimes(
        videoStart: sourceVideo.timeRange.start,
        audioStart: sourceAudio?.timeRange.start
      )
      try destinationVideo.insertTimeRange(
        sourceVideo.timeRange,
        of: sourceVideo,
        at: insertionTimes.video
      )
      destinationVideo.preferredTransform = sourceVideo.preferredTransform

      if let sourceAudio,
        let destinationAudio = composition.addMutableTrack(
          withMediaType: .audio,
          preferredTrackID: kCMPersistentTrackID_Invalid
        )
      {
        try destinationAudio.insertTimeRange(
          sourceAudio.timeRange,
          of: sourceAudio,
          at: insertionTimes.audio ?? .zero
        )
      }
    } catch {
      return completion(.failure(Failure(message: error.localizedDescription)))
    }

    guard
      let exportSession = AVAssetExportSession(
        asset: composition,
        // Passthrough preserves the source's HEVC codec and container
        // metadata. Buzz accepts only canonical H.264/AAC MP4s with no
        // metadata channels, so re-encode instead of copying the movie.
        presetName: AVAssetExportPresetMediumQuality
      )
    else {
      return completion(.failure(Failure(message: "Unable to create export session.")))
    }

    let outputURL = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString)
      .appendingPathExtension("mp4")

    exportSession.outputURL = outputURL
    exportSession.outputFileType = .mp4
    exportSession.shouldOptimizeForNetworkUse = true
    // `forSharing()` intentionally retains playback metadata. The relay
    // rejects every descriptive metadata channel to avoid leaking location or
    // other private information, so write no source metadata at all.
    exportSession.metadata = []
    exportSession.metadataItemFilter = nil

    exportSession.exportAsynchronously {
      switch exportSession.status {
      case .completed:
        do {
          // AVFoundation writes a standard sample-dependency table (`sdtp`).
          // Older Buzz relays mistook that playback-only box for metadata. Keep
          // its size and payload in a `free` box so chunk offsets stay valid and
          // uploads work before those relays receive the validator fix.
          try MP4Canonicalizer.neutralizeSampleDependencyBoxes(at: outputURL)
          completion(.success(outputURL))
        } catch {
          try? FileManager.default.removeItem(at: outputURL)
          completion(.failure(Failure(message: "Unable to canonicalize transcoded video.")))
        }
      default:
        // Clean up partial output on failure.
        try? FileManager.default.removeItem(at: outputURL)
        completion(
          .failure(
            Failure(
              message: exportSession.error?.localizedDescription
                ?? "Video transcoding failed with status \(exportSession.status.rawValue).")))
      }
    }
  }

  static func relativeTrackInsertionTimes(
    videoStart: CMTime,
    audioStart: CMTime?
  ) -> (video: CMTime, audio: CMTime?) {
    guard let audioStart else {
      return (video: .zero, audio: nil)
    }

    let timelineStart =
      CMTimeCompare(audioStart, videoStart) < 0 ? audioStart : videoStart
    return (
      video: CMTimeSubtract(videoStart, timelineStart),
      audio: CMTimeSubtract(audioStart, timelineStart)
    )
  }

  /// A JPEG poster frame for the video at `source`, at most 720 points on a
  /// side. Blocks while it decodes, so call it off the main thread.
  static func poster(source: URL) throws -> Data {
    let asset = AVURLAsset(url: source)
    let generator = AVAssetImageGenerator(asset: asset)
    generator.appliesPreferredTrackTransform = true
    generator.maximumSize = CGSize(width: 720, height: 720)
    generator.requestedTimeToleranceBefore = .positiveInfinity
    generator.requestedTimeToleranceAfter = .positiveInfinity

    let durationSeconds = CMTimeGetSeconds(asset.duration)
    let middleTime =
      durationSeconds.isFinite && durationSeconds > 0
      ? min(durationSeconds / 2, 1)
      : 0
    let candidateTimes = [0, 0.1, middleTime]
    var posterImage: CGImage?
    var lastError: Error?

    for seconds in candidateTimes {
      do {
        posterImage = try generator.copyCGImage(
          at: CMTime(seconds: seconds, preferredTimescale: 600),
          actualTime: nil
        )
        if posterImage != nil { break }
      } catch {
        lastError = error
      }
    }

    guard let posterImage else {
      throw lastError
        ?? NSError(
          domain: "BuzzVideoPoster",
          code: 1,
          userInfo: [NSLocalizedDescriptionKey: "Unable to decode a video frame."]
        )
    }
    guard let jpegData = try MediaSanitizer.encodeJpeg(UIImage(cgImage: posterImage)) else {
      throw NSError(
        domain: "BuzzVideoPoster",
        code: 2,
        userInfo: [NSLocalizedDescriptionKey: "Unable to encode video poster."]
      )
    }
    return jpegData
  }
}
