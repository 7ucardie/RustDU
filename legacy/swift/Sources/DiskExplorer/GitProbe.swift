import Foundation

/// Collects branch / last commit / dirty state for a repository by shelling out to git.
/// Falls back to reading `.git/HEAD` when no git binary is available.
enum GitProbe {
    static let gitExecutable: String? = {
        var candidates = [
            "/opt/homebrew/bin/git",
            "/usr/local/bin/git",
            "/Library/Developer/CommandLineTools/usr/bin/git",
            "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
        ]
        if let dev = run("/usr/bin/xcode-select", ["-p"], timeout: 5)?.trimmingCharacters(in: .whitespacesAndNewlines),
           !dev.isEmpty {
            candidates.append(dev + "/usr/bin/git")
        }
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }
    }()

    static func probe(repoPath: String) -> GitInfo {
        guard let git = gitExecutable else { return fallback(repoPath: repoPath) }
        var info = GitInfo()

        func g(_ args: [String]) -> String? {
            run(git, ["--no-optional-locks", "-C", repoPath] + args, timeout: 60)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }

        if let branch = g(["rev-parse", "--abbrev-ref", "HEAD"]), !branch.isEmpty {
            if branch == "HEAD" {
                info.isDetached = true
                info.branch = g(["rev-parse", "--short", "HEAD"]) ?? "detached"
            } else {
                info.branch = branch
            }
        } else {
            // Probably an empty repository without commits.
            info.branch = fallback(repoPath: repoPath).branch
        }

        if let log = g(["log", "-1", "--format=%ct%x09%s"]), !log.isEmpty {
            let parts = log.split(separator: "\t", maxSplits: 1, omittingEmptySubsequences: false)
            if let ts = parts.first, let seconds = TimeInterval(ts) {
                info.lastCommitDate = Date(timeIntervalSince1970: seconds)
            }
            if parts.count > 1 { info.lastCommitSubject = String(parts[1]) }
        }

        if let status = run(git, ["--no-optional-locks", "-C", repoPath, "status", "--porcelain"], timeout: 120) {
            info.changedFiles = status.split(separator: "\n", omittingEmptySubsequences: true).count
        }

        if let remote = g(["remote", "get-url", "origin"]), !remote.isEmpty {
            info.remote = remote
        }
        return info
    }

    private static func fallback(repoPath: String) -> GitInfo {
        var info = GitInfo()
        info.usedFallback = true
        var gitDir = repoPath + "/.git"
        // `.git` may be a file pointing at the real git dir (worktrees, submodules).
        if let contents = try? String(contentsOfFile: gitDir, encoding: .utf8),
           contents.hasPrefix("gitdir:") {
            let target = contents.dropFirst("gitdir:".count).trimmingCharacters(in: .whitespacesAndNewlines)
            gitDir = target.hasPrefix("/") ? target : repoPath + "/" + target
        }
        if let head = try? String(contentsOfFile: gitDir + "/HEAD", encoding: .utf8) {
            let line = head.trimmingCharacters(in: .whitespacesAndNewlines)
            if line.hasPrefix("ref: refs/heads/") {
                info.branch = String(line.dropFirst("ref: refs/heads/".count))
            } else {
                info.isDetached = true
                info.branch = String(line.prefix(8))
            }
        }
        if let attrs = try? FileManager.default.attributesOfItem(atPath: gitDir + "/HEAD"),
           let date = attrs[.modificationDate] as? Date {
            info.lastCommitDate = date
        }
        return info
    }

    @discardableResult
    static func run(_ executable: String, _ args: [String], timeout: TimeInterval) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = args
        process.environment = [
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": NSHomeDirectory(),
            "GIT_TERMINAL_PROMPT": "0",
            "LC_ALL": "en_US.UTF-8",
        ]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        do { try process.run() } catch { return nil }

        let deadline = DispatchWorkItem { if process.isRunning { process.terminate() } }
        DispatchQueue.global().asyncAfter(deadline: .now() + timeout, execute: deadline)
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        deadline.cancel()

        guard process.terminationStatus == 0 else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
