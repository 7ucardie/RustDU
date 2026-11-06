import SwiftUI

struct GitReposView: View {
    @Environment(ScanModel.self) private var model
    @Binding var selection: NodeRef?
    @State private var sortOrder: [KeyPathComparator<FileNode>] = [KeyPathComparator(\FileNode.size, order: .reverse)]
    @State private var dummyTrash: FileNode?

    private var rows: [FileNode] { model.gitRepos.sorted(using: sortOrder) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("\(model.gitRepos.count) git repositories under \(model.root?.name ?? "")")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Spacer()
                let total = model.gitRepos.reduce(Int64(0)) { $0 + $1.size }
                let gitDirs = model.gitRepos.reduce(Int64(0)) { $0 + ($1.gitDirectory?.size ?? 0) }
                Text("Repos \(Fmt.bytes(total)) · of which .git history \(Fmt.bytes(gitDirs))")
                    .font(.callout)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            Divider()
            if model.gitRepos.isEmpty {
                ContentUnavailableView("No git repositories found", systemImage: "arrow.triangle.branch",
                                       description: Text("Folders that contain a .git entry show up here."))
            } else {
                Table(rows, selection: $selection, sortOrder: $sortOrder) {
                    TableColumn("Repository", value: \.name, comparator: .localizedStandard) { node in
                        HStack(spacing: 6) {
                            Image(systemName: "folder.fill.badge.gearshape").foregroundStyle(Color(nsColor: .systemBlue))
                            Text(node.name).lineLimit(1)
                        }
                    }
                    .width(min: 160, ideal: 220)

                    TableColumn("Branch", value: \.gitSortDate) { node in
                        if let g = node.git {
                            HStack(spacing: 4) {
                                Text(g.branch).lineLimit(1)
                                if g.isDetached { Badge(text: "detached", color: .orange, icon: nil) }
                            }
                        } else {
                            ProgressView().controlSize(.mini)
                        }
                    }
                    .width(min: 100, ideal: 140)

                    TableColumn("Last commit", value: \.gitSortDate) { node in
                        if let d = node.git?.lastCommitDate {
                            DateCell(date: d).help(node.git?.lastCommitSubject ?? "")
                        } else {
                            Text("—").foregroundStyle(.tertiary)
                        }
                    }
                    .width(min: 100, ideal: 120)

                    TableColumn("Changes", value: \.gitSortDate) { node in
                        if let g = node.git {
                            if g.changedFiles > 0 {
                                Text("● \(g.changedFiles)").foregroundStyle(.orange).monospacedDigit()
                            } else if g.usedFallback {
                                Text("?").foregroundStyle(.tertiary)
                            } else {
                                Text("clean").foregroundStyle(.green)
                            }
                        } else {
                            Text("")
                        }
                    }
                    .width(min: 60, ideal: 80)

                    TableColumn("Size", value: \.size) { node in
                        Text(Fmt.bytes(node.size)).monospacedDigit()
                    }
                    .width(min: 80, ideal: 90)

                    TableColumn(".git size", value: \.size) { node in
                        Text(node.gitDirectory.map { Fmt.bytes($0.size) } ?? "—")
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                    }
                    .width(min: 80, ideal: 90)

                    TableColumn("Last activity", value: \.newestModified) { node in
                        DateCell(date: node.newestModified)
                    }
                    .width(min: 100, ideal: 120)

                    TableColumn("Remote", value: \.path) { node in
                        Text(node.git?.remote ?? "")
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    .width(min: 160, ideal: 260)

                    TableColumn("Path", value: \.path) { node in
                        Text(node.path).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    }
                    .width(min: 160, ideal: 300)
                }
                .contextMenu(forSelectionType: NodeRef.self) { refs in
                    if let ref = refs.first {
                        NodeContextMenu(node: ref.node, pendingTrash: $dummyTrash)
                    }
                } primaryAction: { refs in
                    if let ref = refs.first { model.reveal(ref.node) }
                }
            }
        }
    }
}
