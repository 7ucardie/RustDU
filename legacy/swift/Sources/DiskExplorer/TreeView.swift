import SwiftUI

struct TreeView: View {
    @Environment(ScanModel.self) private var model
    @Binding var selection: NodeRef?
    @Binding var pendingTrash: FileNode?
    @State private var customization = TableColumnCustomization<FileNode>()

    var body: some View {
        @Bindable var model = model
        let childKey: KeyPath<FileNode, [FileNode]?> = model.foldersOnly ? \.directoryChildren : \.children

        Table(model.topLevel, children: childKey, selection: $selection,
              sortOrder: $model.sortOrder, columnCustomization: $customization) {
            TableColumn("Name", value: \.name, comparator: .localizedStandard) { node in
                NameCell(node: node)
            }
            .width(min: 200, ideal: 320)
            .customizationID("name")
            .disabledCustomizationBehavior(.visibility)

            TableColumn("Size", value: \.size) { node in
                SizeCell(node: node)
            }
            .width(min: 150, ideal: 160)
            .customizationID("size")

            TableColumn("% of parent", value: \.percentOfParent) { node in
                Text(Fmt.percent(node.percentOfParent))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            .width(min: 70, ideal: 80)
            .customizationID("percent")

            TableColumn("Items", value: \.itemCount) { node in
                Text(node.isDirectory ? Fmt.count(node.itemCount) : "")
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            .width(min: 60, ideal: 80)
            .customizationID("items")

            TableColumn("Last activity", value: \.newestModified) { node in
                DateCell(date: node.newestModified)
            }
            .width(min: 100, ideal: 120)
            .customizationID("activity")

            TableColumn("Modified", value: \.modified) { node in
                DateCell(date: node.modified)
            }
            .width(min: 100, ideal: 120)
            .customizationID("modified")
            .defaultVisibility(.hidden)

            TableColumn("Permissions", value: \.permissionString) { node in
                Text(node.permissionString)
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle(node.isWritableByMe ? .primary : .secondary)
                    .help(node.isWritableByMe ? "Octal \(node.octalString)" : "Octal \(node.octalString) — not writable by you")
            }
            .width(min: 100, ideal: 105)
            .customizationID("permissions")

            TableColumn("Owner", value: \.ownerString) { node in
                Text(node.ownerString).foregroundStyle(.secondary)
            }
            .width(min: 90, ideal: 120)
            .customizationID("owner")

            TableColumn("Git", value: \.gitSortDate) { node in
                GitCell(node: node)
            }
            .width(min: 140, ideal: 220)
            .customizationID("git")
        }
        .id(model.treeVersion)
        .contextMenu(forSelectionType: NodeRef.self) { refs in
            if let ref = refs.first {
                NodeContextMenu(node: ref.node, pendingTrash: $pendingTrash)
            }
        } primaryAction: { refs in
            if let ref = refs.first, !ref.node.isDirectory {
                model.reveal(ref.node)
            }
        }
    }
}
