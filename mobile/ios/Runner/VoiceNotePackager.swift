import AVFoundation
import Flutter

enum VoiceNotePackager {
    static let videoEnvelopeTimeout: TimeInterval = 30
    static let exportTimeout: TimeInterval = 30

    static func package(
        sourcePath: String,
        result: @escaping FlutterResult
    ) {
        let sourceAsset = AVURLAsset(url: URL(fileURLWithPath: sourcePath))
        guard let sourceAudio = sourceAsset.tracks(withMediaType: .audio).first else {
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "The recording does not contain an audio track.",
                    details: nil
                )
            )
            return
        }

        let duration = sourceAudio.timeRange.duration
        guard duration.isValid, duration.isNumeric, CMTimeCompare(duration, .zero) > 0 else {
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "The recording has no playable audio.",
                    details: nil
                )
            )
            return
        }

        Self.makeVoiceNoteVideoTrack(duration: duration) { trackResult in
            switch trackResult {
            case let .failure(error):
                result(
                    FlutterError(
                        code: "transcode_failed",
                        message: "Unable to prepare voice note for upload.",
                        details: error.localizedDescription
                    )
                )
            case let .success(videoURL):
                exportVoiceNoteEnvelope(
                    sourceURL: URL(fileURLWithPath: sourcePath),
                    duration: duration,
                    videoURL: videoURL,
                    result: result
                )
            }
        }
    }

    private static func makeVoiceNoteVideoTrack(
        duration: CMTime,
        completion: @escaping (Result<URL, Error>) -> Void
    ) {
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("mp4")

        do {
            let writer = try AVAssetWriter(outputURL: outputURL, fileType: .mp4)
            let input = AVAssetWriterInput(
                mediaType: .video,
                outputSettings: [
                    AVVideoCodecKey: AVVideoCodecType.h264,
                    AVVideoWidthKey: 16,
                    AVVideoHeightKey: 16,
                    AVVideoCompressionPropertiesKey: [
                        AVVideoAverageBitRateKey: 8000,
                        AVVideoExpectedSourceFrameRateKey: 1,
                        AVVideoMaxKeyFrameIntervalKey: 1,
                    ],
                ]
            )
            input.expectsMediaDataInRealTime = false
            guard writer.canAdd(input) else {
                throw NSError(
                    domain: "BuzzVoiceNote",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "Unable to create the video envelope."]
                )
            }
            writer.add(input)
            let adaptor = AVAssetWriterInputPixelBufferAdaptor(
                assetWriterInput: input,
                sourcePixelBufferAttributes: [
                    kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                    kCVPixelBufferWidthKey as String: 16,
                    kCVPixelBufferHeightKey as String: 16,
                ]
            )
            guard writer.startWriting() else {
                throw writer.error
                    ?? NSError(
                        domain: "BuzzVoiceNote",
                        code: 2,
                        userInfo: [NSLocalizedDescriptionKey: "Unable to start the video envelope."]
                    )
            }
            writer.startSession(atSourceTime: .zero)

            let queue = DispatchQueue(label: "xyz.block.buzz.voice-note-envelope")
            var completed = false
            func complete(_ trackResult: Result<URL, Error>) {
                guard !completed else { return }
                completed = true
                if case .failure = trackResult {
                    writer.cancelWriting()
                    try? FileManager.default.removeItem(at: outputURL)
                }
                completion(trackResult)
            }
            queue.asyncAfter(deadline: .now() + videoEnvelopeTimeout) {
                complete(
                    .failure(
                        NSError(
                            domain: "BuzzVoiceNote",
                            code: 9,
                            userInfo: [NSLocalizedDescriptionKey: "Video envelope generation timed out."]
                        )
                    )
                )
            }
            var appendedFrames = false
            input.requestMediaDataWhenReady(on: queue) {
                guard !completed, !appendedFrames, input.isReadyForMoreMediaData else { return }
                appendedFrames = true
                guard
                    let pool = adaptor.pixelBufferPool,
                    let buffer = Self.makeBlackPixelBuffer(pool: pool),
                    adaptor.append(buffer, withPresentationTime: .zero),
                    adaptor.append(buffer, withPresentationTime: duration)
                else {
                    complete(
                        .failure(
                            writer.error
                                ?? NSError(
                                    domain: "BuzzVoiceNote",
                                    code: 3,
                                    userInfo: [NSLocalizedDescriptionKey: "Unable to write the video envelope."]
                                )
                        )
                    )
                    return
                }
                input.markAsFinished()
                writer.endSession(atSourceTime: duration)
                writer.finishWriting {
                    queue.async {
                        if writer.status == .completed {
                            complete(.success(outputURL))
                        } else {
                            complete(
                                .failure(
                                    writer.error
                                        ?? NSError(
                                            domain: "BuzzVoiceNote",
                                            code: 4,
                                            userInfo: [NSLocalizedDescriptionKey: "Unable to finish the video envelope."]
                                        )
                                )
                            )
                        }
                    }
                }
            }
        } catch {
            try? FileManager.default.removeItem(at: outputURL)
            completion(.failure(error))
        }
    }

    private static func makeBlackPixelBuffer(pool: CVPixelBufferPool) -> CVPixelBuffer? {
        var pixelBuffer: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &pixelBuffer) == kCVReturnSuccess,
              let pixelBuffer
        else {
            return nil
        }
        CVPixelBufferLockBaseAddress(pixelBuffer, [])
        if let baseAddress = CVPixelBufferGetBaseAddress(pixelBuffer) {
            memset(baseAddress, 0, CVPixelBufferGetDataSize(pixelBuffer))
        }
        CVPixelBufferUnlockBaseAddress(pixelBuffer, [])
        return pixelBuffer
    }

    private static func exportVoiceNoteEnvelope(
        sourceURL: URL,
        duration: CMTime,
        videoURL: URL,
        result: @escaping FlutterResult
    ) {
        // Reload the recording here so its AVAsset stays alive for the entire
        // composition insert. Keeping only an AVAssetTrack across the asynchronous
        // video-envelope write can leave the track detached from its source asset
        // on physical devices.
        let sourceAsset = AVURLAsset(url: sourceURL)
        let videoAsset = AVURLAsset(url: videoURL)
        let composition = AVMutableComposition()
        do {
            guard
                let sourceAudio = sourceAsset.tracks(withMediaType: .audio).first,
                let sourceVideo = videoAsset.tracks(withMediaType: .video).first,
                let destinationVideo = composition.addMutableTrack(
                    withMediaType: .video,
                    preferredTrackID: kCMPersistentTrackID_Invalid
                ),
                let destinationAudio = composition.addMutableTrack(
                    withMediaType: .audio,
                    preferredTrackID: kCMPersistentTrackID_Invalid
                )
            else {
                throw NSError(
                    domain: "BuzzVoiceNote",
                    code: 5,
                    userInfo: [NSLocalizedDescriptionKey: "Unable to assemble the voice note envelope."]
                )
            }
            let sourceVideoRange = sourceVideo.timeRange
            try destinationVideo.insertTimeRange(sourceVideoRange, of: sourceVideo, at: .zero)
            destinationVideo.scaleTimeRange(
                CMTimeRange(start: .zero, duration: sourceVideoRange.duration),
                toDuration: duration
            )
            try destinationAudio.insertTimeRange(sourceAudio.timeRange, of: sourceAudio, at: .zero)
        } catch {
            try? FileManager.default.removeItem(at: videoURL)
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "Unable to assemble voice note for upload.",
                    details: error.localizedDescription
                )
            )
            return
        }

        guard let exportSession = AVAssetExportSession(
            asset: composition,
            presetName: AVAssetExportPresetMediumQuality
        ) else {
            try? FileManager.default.removeItem(at: videoURL)
            result(
                FlutterError(
                    code: "transcode_failed",
                    message: "Unable to create voice note export session.",
                    details: nil
                )
            )
            return
        }
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("mp4")
        exportSession.outputURL = outputURL
        exportSession.outputFileType = .mp4
        exportSession.shouldOptimizeForNetworkUse = true
        exportSession.metadata = []
        exportSession.metadataItemFilter = nil
        let completionQueue = DispatchQueue(label: "xyz.block.buzz.voice-note-export")
        let completion = VoiceNoteExportCompletion(
            outputURL: outputURL,
            videoURL: videoURL
        )
        completionQueue.asyncAfter(deadline: .now() + exportTimeout) {
            completion.timeout(
                cancel: exportSession.cancelExport,
                deliver: {
                    result(
                        FlutterError(
                            code: "transcode_failed",
                            message: "Voice note packaging timed out.",
                            details: nil
                        )
                    )
                }
            )
        }
        exportSession.exportAsynchronously {
            completionQueue.async {
                completion.exportDidFinish(
                    succeeded: exportSession.status == .completed,
                    deliver: {
                        switch exportSession.status {
                        case .completed:
                            do {
                                try MP4Canonicalizer.neutralizeSampleDependencyBoxes(at: outputURL)
                                result(outputURL.path)
                            } catch {
                                try? FileManager.default.removeItem(at: outputURL)
                                result(
                                    FlutterError(
                                        code: "transcode_failed",
                                        message: "Unable to canonicalize voice note.",
                                        details: error.localizedDescription
                                    )
                                )
                            }
                        default:
                            result(
                                FlutterError(
                                    code: "transcode_failed",
                                    message: "Voice note packaging failed.",
                                    details: exportSession.error?.localizedDescription
                                )
                            )
                        }
                    }
                )
            }
        }
    }
}

/// Separates one-shot Flutter result delivery from asynchronous export cleanup.
///
/// `cancelExport()` does not synchronously join AVFoundation's exporter. A late
/// terminal callback must therefore remove an output recreated after timeout,
/// even though the timeout already delivered the Flutter result.
final class VoiceNoteExportCompletion {
    private let outputURL: URL
    private let videoURL: URL
    private let fileManager: FileManager
    private var delivered = false

    init(
        outputURL: URL,
        videoURL: URL,
        fileManager: FileManager = .default
    ) {
        self.outputURL = outputURL
        self.videoURL = videoURL
        self.fileManager = fileManager
    }

    func timeout(cancel: () -> Void, deliver: () -> Void) {
        guard !delivered else { return }
        delivered = true
        cancel()
        remove(videoURL)
        remove(outputURL)
        deliver()
    }

    func exportDidFinish(succeeded: Bool, deliver: () -> Void) {
        remove(videoURL)
        guard !delivered else {
            remove(outputURL)
            return
        }
        delivered = true
        if !succeeded { remove(outputURL) }
        deliver()
    }

    private func remove(_ url: URL) {
        try? fileManager.removeItem(at: url)
    }
}
