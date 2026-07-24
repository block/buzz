import Darwin
import Foundation

struct FileRecord: Equatable { let path: String; let contents: String; let device: String; let inode: String
    func output() -> AppleInputRecord { .init(fields: ["path": path, "contents": contents, "device": device, "inode": inode]) }
}

final class FileReader {
    private let roots: [URL]; private let maximumBytes: Int; private let maximumFiles: Int
    init(allowlistedRoots: [URL], maximumBytes: Int = 1_048_576, maximumFiles: Int = 100) { roots = allowlistedRoots.map { $0.standardizedFileURL.resolvingSymlinksInPath() }; self.maximumBytes = maximumBytes; self.maximumFiles = maximumFiles }

    func read(paths: [String]) throws -> [FileRecord] {
        guard paths.count <= maximumFiles else { throw AppleInputFailure.invalidBound }
        return try paths.map(readOne)
    }

    private func readOne(path: String) throws -> FileRecord {
        let requested = URL(fileURLWithPath: path).standardizedFileURL
        let canonical = requested.resolvingSymlinksInPath()
        guard let root = roots.first(where: { canonical.path == $0.path || canonical.path.hasPrefix($0.path + "/") }) else { throw AppleInputFailure.forbidden("file is not a regular path in the allowlist") }
        guard canonical.path == requested.path, canonical.path == root.path || canonical.path.hasPrefix(root.path + "/") else { throw AppleInputFailure.forbidden("file canonical path escaped allowlist") }
        let values = try requested.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey, .fileResourceIdentifierKey])
        guard values.isRegularFile == true, let size = values.fileSize, size <= maximumBytes else { throw AppleInputFailure.forbidden("file is not a bounded regular file") }
        var info = stat(); guard lstat(requested.path, &info) == 0 else { throw AppleInputFailure.invalidRequest("file metadata unavailable") }
        let data = try Data(contentsOf: requested, options: .mappedIfSafe)
        guard let contents = String(data: data, encoding: .utf8) else { throw AppleInputFailure.forbidden("file must be UTF-8 text") }
        return .init(path: canonical.path, contents: contents, device: String(info.st_dev), inode: String(info.st_ino))
    }

}
