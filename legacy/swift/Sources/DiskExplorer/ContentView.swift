import SwiftUI

enum ViewTab: String, CaseIterable, Identifiable {
    case tree = "Explorer"
    case largest = "Largest Files"
    case git = "Git Repos"
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .tree: return "list.bullet.indent"
        case .largest: return "chart.bar.fill"
        case .git: return "arrow.triangle.branch"
        }
    }
}

struct ContentView: View {
    @Environment(ScanModel.self) private var model
    @State private var tab: ViewTab = .tree
    @State private var selection: NodeRef?
    @State private var showInspector = true
    @State private var pendingTrash: FileNode?

    private var selectedNode: FileNode? { selection?.node }

    var body: some View {
        @Bindable var model = model
        VStack(spacing: 0) {
            if let root = model.root {
                RootHeader(root: root)
                Divider()
                switch tab {
                case .tree:
                    TreeView(selection: $selection, pendingTrash: $pendingTrash)
                case .largest:
                    LargestFilesView(selection: $selection, pendingTrash: $pendingTrash)
                case .git:
                    GitReposView(selection: $selection)
                }
            } else {
                emptyState
            }
            Divider()
            StatusBar()
        }
        .inspector(isPresented: $showInspector) {
            InspectorView(node: selectedNode ?? model.root, pendingTrash: $pendingTrash)
                .inspectorColumnWidth(min: 260, ideal: 320, max: 480)
        }
        .toolbar { toolbarContent }
        .navigationTitle(model.root?.name ?? "Disk Explorer")
        .navigationSubtitle(model.rootURL?.path ?? "")
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first(where: { (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true }) else {
                return false
            }
            model.scan(url)
            return true
        }
        .onChange(of: model.treeVersion) { _, _ in
            // Drop selection when its node has been detached from the tree.
            if let node = selectedNode, node.parent == nil, node !== model.root { selection = nil }
        }
        .confirmationDialog(
            "Move “\(pendingTrash?.name ?? "")” to the Trash?",
            isPresented: Binding(get: { pendingTrash != nil }, set: { if !$0 { pendingTrash = nil } }),
            titleVisibility: .visible,
            presenting: pendingTrash
        ) { node in
            Button("Move to Trash", role: .destructive) {
                if selection?.node === node { selection = nil }
                model.trash(node)
            }
            Button("Cancel", role: .cancel) {}
        } message: { node in
            Text("\(Fmt.bytes(node.size)) · \(node.isDirectory ? "\(Fmt.count(node.itemCount)) items" : "file")\n\(node.path)")
        }
        .alert("Error", isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })) {
            Button("OK") {}
        } message: {
            Text(model.errorMessage ?? "")
        }
        .task { scanFromCommandLine() }
    }

    private var emptyState: some View {
        VStack(spacing: 16) {
            Spacer()
            if model.state == .scanning {
                ProgressView()
                    .controlSize(.large)
                Text("Scanning \(model.rootURL?.lastPathComponent ?? "")…")
                    .font(.title2)
                Text("\(Fmt.count(model.progressItems)) items · \(Fmt.bytes(model.progressBytes))")
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                Text(model.progressPath)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 600)
                Button("Stop") { model.cancel() }
                    .keyboardShortcut(".")
            } else {
                Image(systemName: "internaldrive")
                    .font(.system(size: 64))
                    .foregroundStyle(.secondary)
                Text("Scan a folder to see where the space is going")
                    .font(.title2)
                Text("Drop a folder here or choose one.")
                    .foregroundStyle(.secondary)
                Button("Open Folder…") { model.chooseFolder() }
                    .keyboardShortcut("o")
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        @Bindable var model = model
        ToolbarItemGroup(placement: .navigation) {
            Button { model.chooseFolder() } label: {
                Label("Open Folder", systemImage: "folder")
            }
            .help("Open a folder to scan (⌘O)")

            if model.state == .scanning {
                Button { model.cancel() } label: {
                    Label("Stop", systemImage: "stop.circle")
                }
                .help("Stop scanning (⌘.)")
            } else {
                Button { model.rescan() } label: {
                    Label("Rescan", systemImage: "arrow.clockwise")
                }
                .disabled(model.rootURL == nil)
                .help("Rescan the current folder (⌘R)")
            }
        }

        ToolbarItem(placement: .principal) {
            Picker("View", selection: $tab) {
                ForEach(ViewTab.allCases) { t in
                    Label(t.rawValue, systemImage: t.icon).tag(t)
                }
            }
            .pickerStyle(.segmented)
            .disabled(model.root == nil)
        }

        ToolbarItemGroup(placement: .primaryAction) {
            if tab == .tree {
                Toggle(isOn: $model.foldersOnly) {
                    Label("Folders only", systemImage: "folder.fill")
                }
                .help("Hide files, show only folders")
            }
            Picker("Size", selection: $model.sizeMode) {
                ForEach(SizeMode.allCases) { m in Text(m.rawValue).tag(m) }
            }
            .help("On disk = allocated blocks (what you free up). Logical = file lengths.")
            .frame(minWidth: 110)

            Button { showInspector.toggle() } label: {
                Label("Inspector", systemImage: "sidebar.right")
            }
            .help("Show or hide the inspector")
        }
    }

    private func scanFromCommandLine() {
        guard model.root == nil, model.state == .idle else { return }
        let args = CommandLine.arguments.dropFirst().filter { !$0.hasPrefix("-") }
        guard let arg = args.first else { return }
        let path = (arg as NSString).expandingTildeInPath
        var isDir: ObjCBool = false
        if FileManager.default.fileExists(atPath: path, isDirectory: &isDir), isDir.boolValue {
            model.scan(URL(fileURLWithPath: path))
        }
    }
}

// MARK: - Header & status bar

struct RootHeader: View {
    let root: FileNode

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "internaldrive.fill")
                .foregroundStyle(.secondary)
            Text(root.path)
                .font(.callout)
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
            Spacer()
            Group {
                stat("Total", Fmt.bytes(root.size))
                stat("Files", Fmt.count(root.fileCount))
                stat("Folders", Fmt.count(root.directoryCount))
                stat("Last activity", Fmt.relative(root.newestModified))
                if root.unreadableCount > 0 {
                    HStack(spacing: 4) {
                        Image(systemName: "lock.fill").foregroundStyle(.red)
                        Text("\(Fmt.count(root.unreadableCount)) unreadable")
                    }
                    .help("Some folders could not be read. Grant Full Disk Access in System Settings › Privacy & Security to include them.")
                }
            }
            .font(.callout)
            .monospacedDigit()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(.bar)
    }

    private func stat(_ label: String, _ value: String) -> some View {
        HStack(spacing: 4) {
            Text(label).foregroundStyle(.secondary)
            Text(value).fontWeight(.medium)
        }
    }
}

struct StatusBar: View {
    @Environment(ScanModel.self) private var model

    var body: some View {
        HStack(spacing: 8) {
            switch model.state {
            case .idle:
                Text("Ready").foregroundStyle(.secondary)
            case .scanning:
                ProgressView().controlSize(.small)
                Text("Scanning… \(Fmt.count(model.progressItems)) items · \(Fmt.bytes(model.progressBytes)) · \(Fmt.duration(model.elapsed))")
                    .monospacedDigit()
                Text(model.progressPath)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            case .done:
                if let root = model.root {
                    Text("\(Fmt.count(root.itemCount)) items · \(Fmt.bytes(root.size)) · scanned in \(Fmt.duration(model.elapsed))")
                        .monospacedDigit()
                    if model.wasCancelled {
                        Text("· stopped early, totals are incomplete").foregroundStyle(.orange)
                    }
                    if !model.gitRepos.isEmpty {
                        let probed = model.gitRepos.filter { $0.git != nil }.count
                        Text("· \(model.gitRepos.count) git repos" + (probed < model.gitRepos.count ? " (reading \(probed)/\(model.gitRepos.count))" : ""))
                            .foregroundStyle(.secondary)
                    }
                } else {
                    Text("Scan failed").foregroundStyle(.red)
                }
            }
            Spacer()
            if GitProbe.gitExecutable == nil, model.root != nil {
                Text("git not found: repo details are approximate")
                    .foregroundStyle(.secondary)
            }
        }
        .font(.caption)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(.bar)
    }
}
