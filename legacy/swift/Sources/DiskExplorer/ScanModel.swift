import Foundation
import AppKit
import Observation

@MainActor
@Observable
final class ScanModel {
    enum State { case idle, scanning, done }

    var rootURL: URL?
    var root: FileNode?
    var state: State = .idle
    var wasCancelled = false
    var errorMessage: String?

    var progressItems = 0
    var progressBytes: Int64 = 0
    var progressPath = ""
    var elapsed: TimeInterval = 0

    var largestFiles: [FileNode] = []
    var gitRepos: [FileNode] = []

    var foldersOnly = false
    /// Bumped whenever the tree is re-ordered so the hierarchical table reloads.
    var treeVersion = 0

    var sizeMode: SizeMode = .allocated {
        didSet {
            guard sizeMode != oldValue, let root else { return }
            root.applySizeMode(sizeMode)
            root.applySort(sortOrder)
            treeVersion += 1
        }
    }

    var sortOrder: [KeyPathComparator<FileNode>] = [KeyPathComparator(\FileNode.size, order: .reverse)] {
        didSet {
            guard let root else { return }
            root.applySort(sortOrder)
            treeVersion += 1
        }
    }

    var topLevel: [FileNode] { root?.children ?? [] }

    @ObservationIgnored private var scanner: DirectoryScanner?
    @ObservationIgnored private var progressTask: Task<Void, Never>?
    @ObservationIgnored private var scanStart = Date()

    // MARK: Actions

    func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.prompt = "Scan"
        panel.message = "Choose a folder to scan"
        if panel.runModal() == .OK, let url = panel.url {
            scan(url)
        }
    }

    func rescan() {
        if let rootURL { scan(rootURL) }
    }

    func cancel() {
        scanner?.cancel()
    }

    func scan(_ url: URL) {
        scanner?.cancel()
        progressTask?.cancel()

        rootURL = url
        root = nil
        largestFiles = []
        gitRepos = []
        errorMessage = nil
        wasCancelled = false
        progressItems = 0
        progressBytes = 0
        progressPath = url.path
        elapsed = 0
        state = .scanning
        scanStart = Date()

        let scanner = DirectoryScanner()
        self.scanner = scanner
        let path = url.path
        let sort = sortOrder
        let mode = sizeMode

        progressTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, self.state == .scanning else { return }
                let s = scanner.snapshot()
                self.progressItems = s.items
                self.progressBytes = s.bytes
                self.progressPath = s.currentPath
                self.elapsed = Date().timeIntervalSince(self.scanStart)
                try? await Task.sleep(for: .milliseconds(150))
            }
        }

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = scanner.scan(rootPath: path)
            var largest: [FileNode] = []
            var repos: [FileNode] = []
            if let result {
                result.applySizeMode(mode)
                result.applySort(sort)
                largest = TreeAnalysis.largestFiles(in: result, limit: 500)
                repos = TreeAnalysis.gitRepositories(in: result)
            }
            let cancelled = scanner.isCancelled
            DispatchQueue.main.async {
                guard let self, self.scanner === scanner else { return }
                self.finish(root: result, largest: largest, repos: repos, cancelled: cancelled)
            }
        }
    }

    private func finish(root: FileNode?, largest: [FileNode], repos: [FileNode], cancelled: Bool) {
        progressTask?.cancel()
        progressTask = nil
        elapsed = Date().timeIntervalSince(scanStart)
        wasCancelled = cancelled
        if let root {
            self.root = root
            largestFiles = largest
            gitRepos = repos
            treeVersion += 1
            probeGit(repos)
        } else {
            errorMessage = "Could not read \(rootURL?.path ?? "folder")."
        }
        state = .done
    }

    private func probeGit(_ repos: [FileNode]) {
        guard !repos.isEmpty else { return }
        DispatchQueue.global(qos: .utility).async {
            DispatchQueue.concurrentPerform(iterations: repos.count) { i in
                let node = repos[i]
                let info = GitProbe.probe(repoPath: node.path)
                DispatchQueue.main.async { node.git = info }
            }
        }
    }

    // MARK: File operations

    func reveal(_ node: FileNode) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: node.path)])
    }

    func copyPath(_ node: FileNode) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(node.path, forType: .string)
    }

    func openInTerminal(_ node: FileNode) {
        let dir = node.isDirectory ? node.path : (node.path as NSString).deletingLastPathComponent
        let url = URL(fileURLWithPath: dir)
        guard let terminal = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Terminal") else { return }
        NSWorkspace.shared.open([url], withApplicationAt: terminal, configuration: NSWorkspace.OpenConfiguration())
    }

    func trash(_ node: FileNode) {
        guard node !== root else { return }
        do {
            try FileManager.default.trashItem(at: URL(fileURLWithPath: node.path), resultingItemURL: nil)
        } catch {
            errorMessage = "Could not move \(node.name) to Trash: \(error.localizedDescription)"
            return
        }
        let prefix = node.path + "/"
        node.removeFromTree()
        largestFiles.removeAll { $0.path == node.path || $0.path.hasPrefix(prefix) }
        gitRepos.removeAll { $0.path == node.path || $0.path.hasPrefix(prefix) }
        treeVersion += 1
    }
}
