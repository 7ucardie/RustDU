import SwiftUI
import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Needed when launched as a bare executable (swift run) instead of a .app bundle.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

@main
struct DiskExplorerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var model = ScanModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(model)
                .frame(minWidth: 980, minHeight: 560)
        }
        .defaultSize(width: 1500, height: 900)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Open Folder…") { model.chooseFolder() }
                    .keyboardShortcut("o")
                Button("Rescan") { model.rescan() }
                    .keyboardShortcut("r")
                    .disabled(model.rootURL == nil || model.state == .scanning)
                Button("Stop Scan") { model.cancel() }
                    .keyboardShortcut(".")
                    .disabled(model.state != .scanning)
            }
        }
    }
}
