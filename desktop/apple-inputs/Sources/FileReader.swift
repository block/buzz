import Darwin
import Foundation

struct FileRecord: Equatable {
    let path, contents, device, inode: String
    func output() -> AppleInputRecord { .init(fields: ["path": path, "contents": contents, "device": device, "inode": inode]) }
}
final class FileReader {
    private struct Root { let requestedPath, canonicalPath: String; let descriptor: Int32 }
    private let roots: [Root]
    private let maximumBytes, maximumFiles: Int
    init(allowlistedRoots: [URL], maximumBytes: Int = 1_048_576, maximumFiles: Int = 100) {
        self.maximumBytes = maximumBytes; self.maximumFiles = maximumFiles
        roots = allowlistedRoots.compactMap {
            let requested = $0.standardizedFileURL.path
            let canonical = $0.standardizedFileURL.resolvingSymlinksInPath().path
            let descriptor = open(canonical, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW)
            return descriptor >= 0 ? Root(requestedPath: requested, canonicalPath: canonical, descriptor: descriptor) : nil
        }
    }
    deinit { for root in roots { close(root.descriptor) } }
    func read(paths: [String]) throws -> Page<FileRecord> {
        guard !paths.isEmpty, paths.count <= maximumFiles else { throw AppleInputFailure.invalidBound }
        let records = try paths.map(readOne)
        return .init(records: records, truncated: false)
    }
    private func readOne(path: String) throws -> FileRecord {
        let requested = URL(fileURLWithPath: path).standardizedFileURL.path
        guard let root = roots.first(where: { requested.hasPrefix($0.requestedPath + "/") || requested.hasPrefix($0.canonicalPath + "/") }) else {
            throw AppleInputFailure.forbidden("file is outside the allowlist")
        }
        let prefix = requested.hasPrefix(root.requestedPath + "/") ? root.requestedPath : root.canonicalPath
        let relative = String(requested.dropFirst(prefix.count + 1))
        let components = relative.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        guard !components.isEmpty, components.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." && $0.utf8.count <= NAME_MAX }) else {
            throw AppleInputFailure.forbidden("invalid file path")
        }
        var parent = dup(root.descriptor)
        guard parent >= 0 else { throw AppleInputFailure.invalidRequest("allowlist root unavailable") }
        defer { if parent >= 0 { close(parent) } }
        for (index, component) in components.enumerated() {
            let flags = O_RDONLY | O_CLOEXEC | O_NOFOLLOW | (index == components.count - 1 ? 0 : O_DIRECTORY)
            let child = openat(parent, component, flags)
            guard child >= 0 else { throw AppleInputFailure.forbidden("symlink or unreadable file rejected") }
            close(parent); parent = child
        }
        var metadata = stat()
        guard fstat(parent, &metadata) == 0, (metadata.st_mode & S_IFMT) == S_IFREG,
              metadata.st_size >= 0, metadata.st_size <= maximumBytes else {
            throw AppleInputFailure.forbidden("file is not a bounded regular file")
        }
        var data = Data()
        data.reserveCapacity(min(maximumBytes + 1, Int(metadata.st_size) + 1))
        var buffer = [UInt8](repeating: 0, count: min(16_384, maximumBytes + 1))
        while data.count <= maximumBytes {
            let count = Darwin.read(parent, &buffer, min(buffer.count, maximumBytes + 1 - data.count))
            guard count >= 0 else { throw AppleInputFailure.invalidRequest("file read failed") }
            if count == 0 { break }
            data.append(buffer, count: count)
        }
        guard data.count <= maximumBytes, let contents = String(data: data, encoding: .utf8) else {
            throw AppleInputFailure.forbidden("file must be bounded UTF-8 text")
        }
        return .init(path: root.canonicalPath + "/" + relative, contents: contents,
                     device: String(metadata.st_dev), inode: String(metadata.st_ino))
    }
}
