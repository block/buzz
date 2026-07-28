import AppKit
import PDFKit
import XCTest

@MainActor
final class PDFExtractorTests: XCTestCase {
    private final class TextView: NSView {
        override func draw(_ dirtyRect: NSRect) {
            super.draw(dirtyRect)
            ("0800 Navigation brief" as NSString).draw(
                at: NSPoint(x: 24, y: 24),
                withAttributes: [.font: NSFont.systemFont(ofSize: 14)]
            )
        }
    }

    func testTextPDFUsesPDFKitTextWithPageNumber() throws {
        let view = TextView(frame: NSRect(x: 0, y: 0, width: 500, height: 500))
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString).appendingPathExtension("pdf")
        try view.dataWithPDF(inside: view.bounds).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        let records = try PDFExtractor(ocrPage: { _ in
            XCTFail("OCR should not run for a text PDF")
            return ("", 0)
        }).extract(path: url.path)

        XCTAssertEqual(records.count, 1)
        XCTAssertEqual(records[0].page, 1)
        XCTAssertTrue(records[0].text.contains("Navigation brief"))
        XCTAssertNil(records[0].confidence)
    }

    func testImageOnlyPageFallsBackToBoundedOCRResult() throws {
        let image = NSImage(size: NSSize(width: 100, height: 100))
        image.lockFocus()
        NSColor.white.setFill()
        NSRect(x: 0, y: 0, width: 100, height: 100).fill()
        image.unlockFocus()
        let document = PDFDocument()
        document.insert(PDFPage(image: image)!, at: 0)
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString).appendingPathExtension("pdf")
        XCTAssertTrue(document.write(to: url))
        defer { try? FileManager.default.removeItem(at: url) }

        let records = try PDFExtractor(ocrPage: { _ in ("Secure for sea", 1.25) })
            .extract(path: url.path)

        XCTAssertEqual(records, [.init(page: 1, text: "Secure for sea", confidence: 1)])
    }
}
