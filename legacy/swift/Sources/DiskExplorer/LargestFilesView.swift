import SwiftUI

struct LargestFilesView: View {
    @Environment(ScanModel.self) private var model
    @Binding var selection: NodeRef?
    @Binding var pendingTrash: FileNode?
    @State private var sortOrder: [KeyPathComparator<FileNode>] = [KeyPathComparator(\FileNode.size, order: .reverse)]

    private var rows: [FileNode] {
        model.largestFiles.sorted(using: sortOrder)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("The \(model.largestFiles.count) largest files under \(model.root?.name ?? "")")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Spacer()
                let total = model.largestFiles.reduce(Int64(0)) { $0 + $1.size }
                if let root = model.root, root.size > 0 {
                    Text("Together \(Fmt.bytes(total)) · \(Fmt.percent(Double(total) / Double(root.size))) of the scan")
                        .font(.callout)
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            Divider()
            Table(rows, selection: $selection, sortOrder: $sortOrder) {
                TableColumn("Name", value: \.name, comparator: .localizedStandard) { node in
                    NameCell(node: node)
                }
                .width(min: 180, ideal: 260)

                TableColumn("Size", value: \.size) { node in
                    SizeCell(node: node, showBar: false)
                }
                .width(min: 90, ideal: 100)

                TableColumn("Modified", value: \.modified) { node in
                    DateCell(date: node.modified)
                }
                .width(min: 100, ideal: 120)

                TableColumn("Permissions", value: \.permissionString) { node in
                    Text(node.permissionString).font(.system(.body, design: .monospaced))
                }
                .width(min: 100, ideal: 105)

                TableColumn("Owner", value: \.ownerString) { node in
                    Text(node.ownerString).foregroundStyle(.secondary)
                }
                .width(min: 90, ideal: 120)

                TableColumn("Folder", value: \.path) { node in
                    Text(relativeFolder(node))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(node.path)
                }
                .width(min: 200, ideal: 400)
            }
            .contextMenu(forSelectionType: NodeRef.self) { refs in
                if let ref = refs.first {
                    NodeContextMenu(node: ref.node, pendingTrash: $pendingTrash)
                }
            } primaryAction: { refs in
                if let ref = refs.first { model.reveal(ref.node) }
            }
        }
    }

    private func relativeFolder(_ node: FileNode) -> String {
        let folder = (node.path as NSString).deletingLastPathComponent
        guard let rootPath = model.root?.path, folder.hasPrefix(rootPath) else { return folder }
        let rel = String(folder.dropFirst(rootPath.count))
        return rel.isEmpty ? "." : "." + rel
    }
}
