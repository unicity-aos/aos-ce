import AppKit
import SwiftUI
import AOSTrayCore

@MainActor
enum TrayApp {
    static func run(store: TrayStore) {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        let delegate = TrayAppDelegate(store: store)
        application.delegate = delegate
        application.run()
    }
}

@MainActor
final class TrayAppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSMenuDelegate {
    private let session: TraySession
    private var statusItem: NSStatusItem?
    private var panel: NSWindow?
    private var panelController: NSHostingController<PanelView>?

    init(store: TrayStore) {
        self.session = TraySession(store: store)
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installStatusItem()
        installPanel()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        session.presentation.lifecycle.terminateAfterLastWindowClosed
    }

    func applicationWillTerminate(_ notification: Notification) {
        // Quit removes this accessory process only. It must not stop AOS.
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if session.presentation.lifecycle.closeWindowHides {
            sender.orderOut(nil)
            return false
        }
        return true
    }

    @objc
    func showRequests(_ sender: Any?) {
        session.show(.requests)
        revealPanel()
    }

    @objc
    func showCapsules(_ sender: Any?) {
        session.show(.capsules)
        revealPanel()
    }

    @objc
    func quitTray(_ sender: Any?) {
        NSApp.terminate(nil)
    }

    private func installStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = item.button {
            button.title = session.presentation.statusItemTitle
            button.toolTip = session.presentation.statusItemAccessibility
            button.setAccessibilityLabel(session.presentation.statusItemAccessibility)
        }
        item.menu = makeMenu()
        statusItem = item
    }

    private func makeMenu() -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false

        let requests = NSMenuItem(
            title: ProductIdentity.requestsMenuTitle,
            action: #selector(showRequests(_:)),
            keyEquivalent: ""
        )
        requests.target = self
        requests.setAccessibilityLabel("Open Requests")
        menu.addItem(requests)

        let capsules = NSMenuItem(
            title: ProductIdentity.capsulesMenuTitle,
            action: #selector(showCapsules(_:)),
            keyEquivalent: ""
        )
        capsules.target = self
        capsules.setAccessibilityLabel("Open Capsules")
        menu.addItem(capsules)

        menu.addItem(.separator())

        let quit = NSMenuItem(
            title: ProductIdentity.quitMenuTitle,
            action: #selector(quitTray(_:)),
            keyEquivalent: "q"
        )
        quit.target = self
        quit.setAccessibilityLabel(ProductIdentity.quitAccessibility)
        menu.addItem(quit)
        return menu
    }

    private func installPanel() {
        let controller = NSHostingController(rootView: PanelView(session: session))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 440, height: 520),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = ProductIdentity.name
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.contentViewController = controller
        window.setContentSize(NSSize(width: 440, height: 520))
        window.center()
        panelController = controller
        panel = window
    }

    private func revealPanel() {
        guard let panel else { return }
        panelController?.rootView = PanelView(session: session)
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
}
