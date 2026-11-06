// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "DiskExplorer",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "DiskExplorer",
            path: "Sources/DiskExplorer"
        )
    ]
)
