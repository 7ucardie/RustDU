# Disk Explorer

A native macOS app (SwiftUI, macOS 14+) that scans a folder and shows where the disk space is going.

## What it shows

**Explorer tab** – a sortable outline of the scanned folder:

| Column | Meaning |
| --- | --- |
| Name | Icon, name, `git` badge with branch, lock when unreadable, `zzz` when nothing inside changed for a year |
| Size | On-disk (allocated) or logical size, with a bar relative to the parent folder |
| % of parent | Share of the parent folder |
| Items | Files + folders inside |
| Last activity | Newest modification date anywhere inside the folder |
| Modified | The folder/file's own modification date (hidden by default, right-click the header to show) |
| Permissions | `drwxr-xr-x` style mode; dimmed when you cannot write to it; tooltip shows octal |
| Owner | `user:group` |
| Git | Branch, change count (orange dot) or clean (green check), last commit age |

**Largest Files tab** – the 500 biggest files in the scan with their folder.

**Git Repos tab** – every folder containing `.git`: branch, last commit, dirty state, repo size, `.git` history size, remote.

**Inspector** (right side) – full detail for the selection: both sizes, compression savings, counts, dates, permissions in symbolic and octal form, owner/group with ids, whether you can read/write it, git details, and actions (Reveal in Finder, Copy Path, Open in Terminal, Scan This Folder, Move to Trash).

Right-click any row for the same actions. Double-click a file to reveal it in Finder. Drop a folder on the window to scan it.

Sizes follow Finder: "On disk" is the allocated size (what deleting frees), "Logical" is the byte length. Hard links are counted once.

## Build and run

```sh
swift run DiskExplorer              # debug build, opens the app
swift run DiskExplorer ~/workspaces # scan a folder immediately
```

## Build the .app bundle

```sh
./build-app.sh                       # universal (arm64 + x86_64) release build in ./build/Disk Explorer.app
BUNDLE_ID=com.you.DiskExplorer ARCHS=arm64 VERSION=1.0.0 ./build-app.sh
```

The script ad-hoc signs the bundle so it runs locally. Sign it properly with:

```sh
codesign --force --deep --options runtime --timestamp \
  --entitlements DiskExplorer.entitlements \
  --sign "Developer ID Application: Your Name (TEAMID)" "build/Disk Explorer.app"
codesign --verify --deep --strict --verbose=2 "build/Disk Explorer.app"
```

For notarization zip it with `ditto -c -k --keepParent`, submit with `xcrun notarytool submit --wait`, then `xcrun stapler staple`.

`DiskExplorer.entitlements` deliberately has no App Sandbox: the app needs to read arbitrary folders. Protected locations (Mail, Messages, Photos, Time Machine backups) still need Full Disk Access, granted in System Settings › Privacy & Security › Full Disk Access. Unreadable folders are counted and flagged with a lock icon.

## Project layout

```
Package.swift                  SwiftPM manifest (executable target, macOS 14)
Sources/DiskExplorer/
  DiskExplorerApp.swift        App entry, menu commands
  ScanModel.swift              Main-actor state: scanning, sorting, trash, git probing
  DirectoryScanner.swift       lstat/readdir tree walk, hard-link dedupe, parallel top levels
  FileNode.swift               Tree node model, permission/owner helpers
  GitProbe.swift               Runs git for branch/commit/status, falls back to .git/HEAD
  ContentView.swift            Window layout, toolbar, header, status bar
  TreeView.swift               Hierarchical table
  LargestFilesView.swift       Flat table of biggest files
  GitReposView.swift           Table of repositories
  InspectorView.swift          Detail pane
  Cells.swift                  Reusable cells and context menu
  Formatters.swift             Byte/date formatting
Tools/make-icon.swift          Renders the app icon
build-app.sh                   Produces the signed-ready .app bundle
DiskExplorer.entitlements      Entitlements used for signing
```
