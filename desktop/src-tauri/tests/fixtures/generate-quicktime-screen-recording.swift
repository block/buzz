// Generates the tiny H.264 QuickTime fixture beside this file.
// Run: swift tests/fixtures/generate-quicktime-screen-recording.swift \
//   tests/fixtures/quicktime-screen-recording.mov
import AVFoundation
import CoreVideo
import Foundation

let output = URL(fileURLWithPath: CommandLine.arguments[1])
try? FileManager.default.removeItem(at: output)
let writer = try AVAssetWriter(outputURL: output, fileType: .mov)
let input = AVAssetWriterInput(
    mediaType: .video,
    outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: 64,
        AVVideoHeightKey: 64,
    ]
)
let adaptor = AVAssetWriterInputPixelBufferAdaptor(
    assetWriterInput: input,
    sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: 64,
        kCVPixelBufferHeightKey as String: 64,
    ]
)
writer.add(input)
guard writer.startWriting() else { fatalError("start failed: \(writer.error!)") }
writer.startSession(atSourceTime: .zero)
input.requestMediaDataWhenReady(on: DispatchQueue(label: "fixture")) {
    for frame in 0..<3 {
        while !input.isReadyForMoreMediaData { usleep(1_000) }
        var buffer: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &buffer)
        guard let pixel = buffer else { fatalError("pixel buffer allocation failed") }
        CVPixelBufferLockBaseAddress(pixel, [])
        memset(
            CVPixelBufferGetBaseAddress(pixel),
            Int32(frame * 32),
            CVPixelBufferGetDataSize(pixel)
        )
        CVPixelBufferUnlockBaseAddress(pixel, [])
        guard adaptor.append(
            pixel,
            withPresentationTime: CMTime(value: CMTimeValue(frame), timescale: 30)
        ) else { fatalError("append failed: \(writer.error!)") }
    }
    input.markAsFinished()
    writer.finishWriting {
        guard writer.status == .completed else { fatalError("finish failed: \(writer.error!)") }
        exit(0)
    }
}
dispatchMain()
