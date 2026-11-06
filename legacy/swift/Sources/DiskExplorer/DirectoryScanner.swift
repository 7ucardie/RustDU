import Foundation
import Darwin

/// Walks a directory tree with lstat/readdir, building a `FileNode` tree.
/// Runs on background threads; progress is read through `snapshot()`.
final class DirectoryScanner {
    struct Snapshot {
        var items: Int
        var bytes: Int64
        var currentPath: String
    }

    private struct LinkKey: Hashable {
        let dev: dev_t
        let ino: ino_t
    }

    private let lock = NSLock()
    private var items = 0
    private var bytes: Int64 = 0
    private var currentPath = ""
    private var cancelled = false
    private var seenLinks = Set<LinkKey>()

    /// Directory levels (from the root) whose subfolders are scanned concurrently.
    private let parallelDepth = 2

    var isCancelled: Bool {
        lock.lock(); defer { lock.unlock() }
        return cancelled
    }

    func cancel() {
        lock.lock(); cancelled = true; lock.unlock()
    }

    func snapshot() -> Snapshot {
        lock.lock(); defer { lock.unlock() }
        return Snapshot(items: items, bytes: bytes, currentPath: currentPath)
    }

    func scan(rootPath: String) -> FileNode? {
        var st = stat()
        guard lstat(rootPath, &st) == 0 else { return nil }
        let last = (rootPath as NSString).lastPathComponent
        let root = makeNode(path: rootPath, name: last.isEmpty ? rootPath : last, st: st, parent: nil)
        if root.isDirectory {
            scanChildren(of: root, depth: 0)
        }
        return root
    }

    // MARK: - Internals

    private func makeNode(path: String, name: String, st: stat, parent: FileNode?) -> FileNode {
        let fmt = st.st_mode & mode_t(S_IFMT)
        let kind: FileNode.Kind
        switch fmt {
        case mode_t(S_IFDIR): kind = .directory
        case mode_t(S_IFLNK): kind = .symlink
        case mode_t(S_IFREG): kind = .regular
        default: kind = .other
        }

        var logical = Int64(st.st_size)
        var allocated = Int64(st.st_blocks) * 512
        var duplicate = false
        if kind != .directory && st.st_nlink > 1 {
            let key = LinkKey(dev: st.st_dev, ino: st.st_ino)
            lock.lock()
            if seenLinks.contains(key) { duplicate = true } else { seenLinks.insert(key) }
            lock.unlock()
            if duplicate { logical = 0; allocated = 0 }
        }

        let mtime = Date(timeIntervalSince1970: TimeInterval(st.st_mtimespec.tv_sec)
                         + TimeInterval(st.st_mtimespec.tv_nsec) / 1_000_000_000)

        return FileNode(name: name, path: path, kind: kind, mode: st.st_mode, uid: st.st_uid, gid: st.st_gid,
                        modified: mtime, logicalSize: logical, allocatedSize: allocated,
                        parent: parent, isHardLinkDuplicate: duplicate)
    }

    private func scanChildren(of node: FileNode, depth: Int) {
        guard let dir = opendir(node.path) else {
            node.isUnreadable = true
            node.unreadableCount = 1
            node.children = []
            return
        }

        var entries: [FileNode] = []
        var subdirs: [FileNode] = []
        var fileBytes: Int64 = 0
        var unreadable = 0

        while let entry = readdir(dir) {
            if isCancelled { break }
            let name = withUnsafePointer(to: &entry.pointee.d_name) { ptr in
                ptr.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { String(cString: $0) }
            }
            if name == "." || name == ".." { continue }

            let childPath = node.path == "/" ? "/" + name : node.path + "/" + name
            var st = stat()
            guard lstat(childPath, &st) == 0 else {
                unreadable += 1
                continue
            }
            let child = makeNode(path: childPath, name: name, st: st, parent: node)
            if name == ".git" { node.isGitRepo = true }
            entries.append(child)
            if child.isDirectory {
                subdirs.append(child)
            } else {
                fileBytes += child.allocatedSize
            }
        }
        closedir(dir)

        node.children = entries

        lock.lock()
        items += entries.count
        bytes += fileBytes
        currentPath = node.path
        lock.unlock()

        if depth < parallelDepth && subdirs.count > 1 {
            DispatchQueue.concurrentPerform(iterations: subdirs.count) { i in
                self.scanChildren(of: subdirs[i], depth: depth + 1)
            }
        } else {
            for sub in subdirs {
                if isCancelled { break }
                scanChildren(of: sub, depth: depth + 1)
            }
        }

        // Aggregate subtree totals.
        var logical = node.ownLogicalSize
        var allocated = node.ownAllocatedSize
        var files = 0
        var dirs = 0
        var newest = node.modified
        for child in entries {
            logical += child.logicalSize
            allocated += child.allocatedSize
            unreadable += child.unreadableCount
            if child.isDirectory {
                dirs += 1 + child.directoryCount
                files += child.fileCount
            } else {
                files += 1
            }
            if child.newestModified > newest { newest = child.newestModified }
        }
        node.logicalSize = logical
        node.allocatedSize = allocated
        node.size = allocated
        node.fileCount = files
        node.directoryCount = dirs
        node.unreadableCount = unreadable
        node.newestModified = newest
    }
}

// MARK: - Post-scan analysis (background)

enum TreeAnalysis {
    static func largestFiles(in root: FileNode, limit: Int) -> [FileNode] {
        var files: [FileNode] = []
        files.reserveCapacity(4096)
        collectFiles(root, into: &files)
        files.sort { $0.allocatedSize > $1.allocatedSize }
        return Array(files.prefix(limit))
    }

    private static func collectFiles(_ node: FileNode, into out: inout [FileNode]) {
        guard let kids = node.children else { return }
        for k in kids {
            if k.isDirectory { collectFiles(k, into: &out) } else { out.append(k) }
        }
    }

    static func gitRepositories(in root: FileNode) -> [FileNode] {
        var repos: [FileNode] = []
        collectRepos(root, into: &repos)
        repos.sort { $0.allocatedSize > $1.allocatedSize }
        return repos
    }

    private static func collectRepos(_ node: FileNode, into out: inout [FileNode]) {
        if node.isGitRepo { out.append(node) }
        guard let kids = node.children else { return }
        for k in kids where k.isDirectory && k.name != ".git" {
            collectRepos(k, into: &out)
        }
    }
}
