import Foundation
import Observation

/// Hashable handle used as the table selection ID; gives direct access to the node.
struct NodeRef: Hashable {
    let node: FileNode
    static func == (lhs: NodeRef, rhs: NodeRef) -> Bool { lhs.node === rhs.node }
    func hash(into hasher: inout Hasher) { hasher.combine(ObjectIdentifier(node)) }
}

enum SizeMode: String, CaseIterable, Identifiable {
    case allocated = "On disk"
    case logical = "Logical"
    var id: String { rawValue }
}

struct GitInfo {
    var branch: String = "?"
    var isDetached = false
    var lastCommitDate: Date?
    var lastCommitSubject: String?
    var changedFiles = 0
    var remote: String?
    var usedFallback = false
}

@Observable
final class FileNode: Identifiable {
    enum Kind { case directory, regular, symlink, other }

    let name: String
    let path: String
    let kind: Kind
    let mode: mode_t
    let uid: uid_t
    let gid: gid_t
    let modified: Date
    let ownLogicalSize: Int64
    let ownAllocatedSize: Int64
    let isHardLinkDuplicate: Bool
    @ObservationIgnored weak var parent: FileNode?

    var logicalSize: Int64
    var allocatedSize: Int64
    /// Size shown in the UI, depends on the selected `SizeMode`.
    var size: Int64
    /// Newest modification date anywhere in the subtree (== `modified` for files).
    var newestModified: Date
    var fileCount = 0
    var directoryCount = 0
    var unreadableCount = 0
    var isUnreadable = false
    var isGitRepo = false
    var git: GitInfo?
    var children: [FileNode]?

    init(name: String, path: String, kind: Kind, mode: mode_t, uid: uid_t, gid: gid_t,
         modified: Date, logicalSize: Int64, allocatedSize: Int64, parent: FileNode?, isHardLinkDuplicate: Bool) {
        self.name = name
        self.path = path
        self.kind = kind
        self.mode = mode
        self.uid = uid
        self.gid = gid
        self.modified = modified
        self.ownLogicalSize = logicalSize
        self.ownAllocatedSize = allocatedSize
        self.logicalSize = logicalSize
        self.allocatedSize = allocatedSize
        self.size = allocatedSize
        self.newestModified = modified
        self.parent = parent
        self.isHardLinkDuplicate = isHardLinkDuplicate
    }

    var id: NodeRef { NodeRef(node: self) }

    // MARK: Derived values

    var isDirectory: Bool { kind == .directory }
    var itemCount: Int { fileCount + directoryCount }
    var directoryChildren: [FileNode]? { children?.filter { $0.isDirectory } }

    var percentOfParent: Double {
        guard let parent, parent.size > 0 else { return 1 }
        return Double(size) / Double(parent.size)
    }

    /// Nothing in the subtree touched for more than a year.
    var isStale: Bool { newestModified < Date().addingTimeInterval(-365 * 86400) }

    var gitSortDate: Date { git?.lastCommitDate ?? .distantPast }
    var gitDirectory: FileNode? { children?.first { $0.name == ".git" } }

    var permissionString: String { Permissions.symbolic(mode) }
    var octalString: String { Permissions.octal(mode) }
    var ownerName: String { Accounts.user(uid) }
    var groupName: String { Accounts.group(gid) }
    var ownerString: String { "\(ownerName):\(groupName)" }
    var isWritableByMe: Bool { Darwin.access(path, W_OK) == 0 }
    var isReadableByMe: Bool { Darwin.access(path, R_OK) == 0 }

    var kindDescription: String {
        switch kind {
        case .directory: return isGitRepo ? "Folder (git repository)" : "Folder"
        case .regular: return "File"
        case .symlink: return "Symbolic link"
        case .other: return "Special file"
        }
    }

    var fileExtension: String { (name as NSString).pathExtension.lowercased() }

    // MARK: Tree mutation (main actor, after scan)

    func applySizeMode(_ mode: SizeMode) {
        size = mode == .allocated ? allocatedSize : logicalSize
        children?.forEach { $0.applySizeMode(mode) }
    }

    func applySort(_ comparators: [KeyPathComparator<FileNode>]) {
        guard var kids = children, !kids.isEmpty else { return }
        kids.sort(using: comparators)
        children = kids
        kids.forEach { $0.applySort(comparators) }
    }

    /// Detach from parent and subtract this subtree's totals all the way up.
    func removeFromTree() {
        let files = isDirectory ? fileCount : 1
        let dirs = isDirectory ? directoryCount + 1 : 0
        var p = parent
        while let node = p {
            node.logicalSize -= logicalSize
            node.allocatedSize -= allocatedSize
            node.size -= size
            node.fileCount -= files
            node.directoryCount -= dirs
            node.unreadableCount -= unreadableCount
            p = node.parent
        }
        parent?.children?.removeAll { $0 === self }
        parent = nil
    }
}

// MARK: - Helpers

enum Permissions {
    static func symbolic(_ mode: mode_t) -> String {
        var s = ""
        switch mode & mode_t(S_IFMT) {
        case mode_t(S_IFDIR): s = "d"
        case mode_t(S_IFLNK): s = "l"
        case mode_t(S_IFREG): s = "-"
        case mode_t(S_IFSOCK): s = "s"
        case mode_t(S_IFIFO): s = "p"
        case mode_t(S_IFCHR): s = "c"
        case mode_t(S_IFBLK): s = "b"
        default: s = "?"
        }
        func triplet(_ r: mode_t, _ w: mode_t, _ x: mode_t, special: mode_t, specialChar: Character) -> String {
            var t = ""
            t.append(mode & r != 0 ? "r" : "-")
            t.append(mode & w != 0 ? "w" : "-")
            let hasX = mode & x != 0
            let hasSpecial = mode & special != 0
            if hasSpecial {
                t.append(hasX ? specialChar : Character(specialChar.uppercased()))
            } else {
                t.append(hasX ? "x" : "-")
            }
            return t
        }
        s += triplet(S_IRUSR, S_IWUSR, S_IXUSR, special: S_ISUID, specialChar: "s")
        s += triplet(S_IRGRP, S_IWGRP, S_IXGRP, special: S_ISGID, specialChar: "s")
        s += triplet(S_IROTH, S_IWOTH, S_IXOTH, special: S_ISVTX, specialChar: "t")
        return s
    }

    static func octal(_ mode: mode_t) -> String {
        let v = String(Int(mode & 0o7777), radix: 8)
        return String(repeating: "0", count: max(0, 4 - v.count)) + v
    }
}

enum Accounts {
    private static let lock = NSLock()
    private static var users: [uid_t: String] = [:]
    private static var groups: [gid_t: String] = [:]

    static func user(_ uid: uid_t) -> String {
        lock.lock(); defer { lock.unlock() }
        if let cached = users[uid] { return cached }
        let name: String
        if let pw = getpwuid(uid) { name = String(cString: pw.pointee.pw_name) } else { name = "\(uid)" }
        users[uid] = name
        return name
    }

    static func group(_ gid: gid_t) -> String {
        lock.lock(); defer { lock.unlock() }
        if let cached = groups[gid] { return cached }
        let name: String
        if let gr = getgrgid(gid) { name = String(cString: gr.pointee.gr_name) } else { name = "\(gid)" }
        groups[gid] = name
        return name
    }
}
