import SwiftUI

@main
struct MuMenuBarApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    
    var body: some Scene {
        Settings {
            EmptyView()
        }
    }
}

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem?
    var popover = NSPopover()
    var playerController: PlayerController?
    
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Hide dock icon
        NSApplication.shared.setActivationPolicy(.accessory)
        
        // Create status item
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        
        if let button = statusItem?.button {
            button.image = NSImage(systemSymbolName: "music.note", accessibilityDescription: "Mu Player")
            button.action = #selector(togglePopover)
            button.target = self
        }
        
        // Setup popover
        let menuBarView = MenuBarView()
        playerController = menuBarView.player
        playerController?.appDelegate = self
        popover.contentViewController = NSHostingController(rootView: menuBarView)
        popover.behavior = .transient
    }
    
    @objc func togglePopover() {
        if popover.isShown {
            popover.performClose(nil)
        } else {
            showPopover()
        }
    }
    
    func showPopover() {
        if let button = statusItem?.button {
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        }
    }
}
