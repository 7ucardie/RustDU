import SwiftUI

struct InspectorView: View {
    @Environment(ScanModel.self) private var model
    let node: FileNode?
    @Binding var pendingTrash: FileNode?

    var body: some View {
        if let node {
            Form {
                Section {
                    HStack(alignment: .top, spacing: 10) {
                        NameCell(node: node).font(.headline).lineLimit(2)
                    }
                    Text(node.path)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    LabeledContent("Kind", value: node.kindDescription)
                }

                Section("Size") {
                    LabeledContent("On disk", value: Fmt.bytes(node.allocatedSize))
                    LabeledContent("Logical", value: Fmt.bytes(node.logicalSize))
                    if node.allocatedSize < node.logicalSize, node.logicalSize > 0 {
                        LabeledContent("Saved by compression/sparse",
                                       value: Fmt.bytes(node.logicalSize - node.allocatedSize))
                    }
                    if node.parent != nil {
                        LabeledContent("Of parent", value: Fmt.percent(node.percentOfParent))
                    }
                    if let root = model.root, node !== root, root.size > 0 {
                        LabeledContent("Of scan", value: Fmt.percent(Double(node.size) / Double(root.size)))
                    }
                    if node.isDirectory {
                        LabeledContent("Files", value: Fmt.count(node.fileCount))
                        LabeledContent("Folders", value: Fmt.count(node.directoryCount))
                        if node.unreadableCount > 0 {
                            LabeledContent("Unreadable items", value: Fmt.count(node.unreadableCount))
                                .foregroundStyle(.red)
                        }
                    }
                    if node.isHardLinkDuplicate {
                        Text("Hard link to a file already counted elsewhere; size counted as 0.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }

                Section("Dates") {
                    LabeledContent("Modified") {
                        VStack(alignment: .trailing) {
                            Text(Fmt.absolute(node.modified))
                            Text(Fmt.relative(node.modified)).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    if node.isDirectory {
                        LabeledContent("Last activity inside") {
                            VStack(alignment: .trailing) {
                                Text(Fmt.absolute(node.newestModified))
                                Text(Fmt.relative(node.newestModified)).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        if node.isStale {
                            Label("Nothing modified in over a year", systemImage: "zzz")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }

                Section("Permissions") {
                    LabeledContent("Mode") {
                        Text("\(node.permissionString)  \(node.octalString)")
                            .font(.system(.body, design: .monospaced))
                    }
                    LabeledContent("Owner", value: "\(node.ownerName) (\(node.uid))")
                    LabeledContent("Group", value: "\(node.groupName) (\(node.gid))")
                    LabeledContent("You can read", value: node.isReadableByMe ? "Yes" : "No")
                    LabeledContent("You can write", value: node.isWritableByMe ? "Yes" : "No")
                }

                if node.isGitRepo {
                    Section("Git repository") {
                        if let g = node.git {
                            LabeledContent("Branch", value: g.branch + (g.isDetached ? " (detached)" : ""))
                            if let d = g.lastCommitDate {
                                LabeledContent("Last commit") {
                                    VStack(alignment: .trailing) {
                                        Text(Fmt.absolute(d))
                                        Text(Fmt.relative(d)).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                            if let s = g.lastCommitSubject, !s.isEmpty {
                                LabeledContent("Message") {
                                    Text(s).multilineTextAlignment(.trailing).lineLimit(3)
                                }
                            }
                            if g.usedFallback {
                                LabeledContent("Working tree", value: "unknown (git not available)")
                            } else {
                                LabeledContent("Working tree",
                                               value: g.changedFiles == 0 ? "Clean" : "\(g.changedFiles) changed/untracked")
                            }
                            if let r = g.remote {
                                LabeledContent("Remote") {
                                    Text(r).lineLimit(2).multilineTextAlignment(.trailing).textSelection(.enabled)
                                }
                            }
                        } else {
                            HStack { ProgressView().controlSize(.small); Text("Reading repository…") }
                        }
                        if let gitDir = node.gitDirectory {
                            LabeledContent(".git folder", value: Fmt.bytes(gitDir.size))
                        }
                    }
                }

                Section {
                    Button { model.reveal(node) } label: { Label("Reveal in Finder", systemImage: "magnifyingglass") }
                    Button { model.copyPath(node) } label: { Label("Copy Path", systemImage: "doc.on.doc") }
                    Button { model.openInTerminal(node) } label: { Label("Open in Terminal", systemImage: "terminal") }
                    if node.isDirectory && node !== model.root {
                        Button { model.scan(URL(fileURLWithPath: node.path)) } label: {
                            Label("Scan This Folder", systemImage: "arrow.down.right.circle")
                        }
                    }
                    Button(role: .destructive) { pendingTrash = node } label: {
                        Label("Move to Trash…", systemImage: "trash")
                    }
                    .disabled(node === model.root)
                }
            }
            .formStyle(.grouped)
        } else {
            ContentUnavailableView("Nothing selected", systemImage: "sidebar.right",
                                   description: Text("Select a folder or file to see its details."))
        }
    }
}
