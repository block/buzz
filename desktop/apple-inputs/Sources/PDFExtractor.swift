import AppKit
import Foundation
import PDFKit
import Vision

struct PDFPageRecord: Codable, Equatable {
    let page: Int
    let text: String
    let confidence: Double?
}

struct PDFExtractor {
    typealias OCRPage = (PDFPage) throws -> (text: String, confidence: Double)

    private let ocrPage: OCRPage

    init(ocrPage: @escaping OCRPage = PDFExtractor.visionOCR) {
        self.ocrPage = ocrPage
    }

    func extract(path: String) throws -> [PDFPageRecord] {
        guard path.hasPrefix("/"), let document = PDFDocument(url: URL(fileURLWithPath: path)) else {
            throw AppleInputFailure.invalidRequest("PDF could not be opened")
        }
        guard document.pageCount <= 2_000 else {
            throw AppleInputFailure.invalidBound
        }
        var records: [PDFPageRecord] = []
        var totalBytes = 0
        for index in 0..<document.pageCount {
            guard let page = document.page(at: index) else { continue }
            let direct = page.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let result: (text: String, confidence: Double?)
            if direct.isEmpty {
                let ocr = try ocrPage(page)
                result = (ocr.text, min(max(ocr.confidence, 0), 1))
            } else {
                result = (direct, nil)
            }
            totalBytes += result.text.utf8.count
            guard totalBytes <= 3 * 1024 * 1024 else {
                throw AppleInputFailure.invalidBound
            }
            records.append(.init(page: index + 1, text: result.text, confidence: result.confidence))
        }
        return records
    }

    private static func visionOCR(page: PDFPage) throws -> (text: String, confidence: Double) {
        let thumbnail = page.thumbnail(of: NSSize(width: 2_000, height: 2_000), for: .mediaBox)
        guard let cgImage = thumbnail.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
            throw AppleInputFailure.invalidRequest("PDF page could not be rendered for OCR")
        }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        try VNImageRequestHandler(cgImage: cgImage).perform([request])
        let observations = request.results ?? []
        let candidates = observations.compactMap { $0.topCandidates(1).first }
        let text = candidates.map(\.string).joined(separator: "\n")
        let confidence = candidates.isEmpty
            ? 0
            : candidates.reduce(0) { $0 + Double($1.confidence) } / Double(candidates.count)
        return (text, confidence)
    }
}
