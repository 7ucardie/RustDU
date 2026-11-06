import SwiftUI

struct NameCell: View {
    let node: FileNode

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: iconName)
                .foregroundStyle(iconColor)
                .frame(width: 16)
            Text(node.name)
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(node.isStale ? .secondary : .primary)
            if node.isGitRepo {
                Badge(text: node.git?.branch ?? "git", color: .purple, icon: "arrow.triangle.branch")
            }
            if node.isUnreadable {
                Image(systemName: "lock.fill")
                    .foregroundStyle(.red)
                    .help("Not readable with the current permissions")
            }
            if node.isHardLinkDuplicate {
                Badge(text: "hard link", color: .gray, icon: nil)
                    .help("Already counted elsewhere (hard link)")
            }
            if node.isStale {
                Image(systemName: "zzz")
                    .foregroundStyle(.tertiary)
                    .help("Nothing in here was modified in the last year")
            }
        }
    }

    private var iconName: String {
        switch node.kind {
        case .directory:
            if node.name == ".git" { return "folder.badge.gearshape" }
            if node.isGitRepo { return "folder.fill.badge.gearshape" }
            if node.fileExtension == "app" { return "app.fill" }
            return "folder.fill"
        case .symlink: return "link"
        case .other: return "questionmark.square.dashed"
        case .regular:
            switch node.fileExtension {
            case "jpg", "jpeg", "png", "gif", "heic", "tiff", "webp", "svg", "raw", "psd": return "photo"
            case "mp4", "mov", "mkv", "avi", "m4v", "webm": return "film"
            case "mp3", "m4a", "wav", "flac", "aac", "aiff": return "music.note"
            case "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "dmg", "iso", "pkg": return "doc.zipper"
            case "swift", "py", "js", "ts", "tsx", "jsx", "rs", "go", "c", "h", "cpp", "java", "kt", "rb", "php", "sh", "html", "css", "json", "yaml", "yml", "toml", "xml": return "chevron.left.forwardslash.chevron.right"
            case "pdf": return "doc.richtext"
            case "sqlite", "db", "sqlite3", "realm": return "cylinder.split.1x2"
            case "log", "txt", "md": return "doc.text"
            default: return "doc"
            }
        }
    }

    private var iconColor: Color {
        switch node.kind {
        case .directory: return node.isStale ? Color.secondary : Color(nsColor: .systemBlue)
        case .symlink: return .teal
        default: return .secondary
        }
    }
}

struct Badge: View {
    let text: String
    let color: Color
    let icon: String?

    var body: some View {
        HStack(spacing: 2) {
            if let icon { Image(systemName: icon).font(.system(size: 8, weight: .bold)) }
            Text(text).font(.system(size: 10, weight: .semibold))
        }
        .lineLimit(1)
        .padding(.horizontal, 5)
        .padding(.vertical, 1)
        .background(color.opacity(0.15), in: Capsule())
        .foregroundStyle(color)
    }
}

struct SizeCell: View {
    let node: FileNode
    var showBar = true

    var body: some View {
        HStack(spacing: 8) {
            if showBar {
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(Color.primary.opacity(0.08))
                        Capsule().fill(barColor)
                            .frame(width: max(2, geo.size.width * CGFloat(min(1, node.percentOfParent))))
                    }
                }
                .frame(width: 70, height: 7)
            }
            Text(Fmt.bytes(node.size))
                .monospacedDigit()
                .frame(minWidth: 64, alignment: .trailing)
                .help(Fmt.exactBytes(node.size))
        }
    }

    private var barColor: Color {
        let p = node.percentOfParent
        if p >= 0.5 { return .red }
        if p >= 0.2 { return .orange }
        if p >= 0.05 { return .accentColor }
        return .accentColor.opacity(0.6)
    }
}

struct DateCell: View {
    let date: Date
    var body: some View {
        Text(Fmt.relative(date))
            .foregroundStyle(date < Date().addingTimeInterval(-365 * 86400) ? .secondary : .primary)
            .help(Fmt.absolute(date))
    }
}

struct GitCell: View {
    let node: FileNode

    var body: some View {
        if node.isGitRepo {
            if let g = node.git {
                HStack(spacing: 6) {
                    Image(systemName: "arrow.triangle.branch").foregroundStyle(.purple)
                    Text(g.branch).lineLimit(1)
                    if g.changedFiles > 0 {
                        Text("● \(g.changedFiles)")
                            .foregroundStyle(.orange)
                            .help("\(g.changedFiles) changed or untracked files")
                    } else if !g.usedFallback {
                        Image(systemName: "checkmark.circle").foregroundStyle(.green).help("Working tree clean")
                    }
                    if let d = g.lastCommitDate {
                        Text(Fmt.relative(d))
                            .foregroundStyle(.secondary)
                            .help("Last commit: \(Fmt.absolute(d))\n\(g.lastCommitSubject ?? "")")
                    }
                }
            } else {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text("reading…").foregroundStyle(.secondary)
                }
            }
        } else {
            Text("")
        }
    }
}

/// Context menu shared by the tables.
struct NodeContextMenu: View {
    @Environment(ScanModel.self) private var model
    let node: FileNode
    @Binding var pendingTrash: FileNode?

    var body: some View {
        Button("Reveal in Finder") { model.reveal(node) }
        Button("Copy Path") { model.copyPath(node) }
        Button("Open in Terminal") { model.openInTerminal(node) }
        if node.isDirectory {
            Divider()
            Button("Scan This Folder") { model.scan(URL(fileURLWithPath: node.path)) }
        }
        Divider()
        Button("Move to Trash…", role: .destructive) { pendingTrash = node }
            .disabled(node === model.root)
    }
}
