import AppKit
import SwiftUI
import AOSTrayCore

@MainActor
enum TrayApp {
    static func run(store: TrayStore, socketPath: String? = nil,
                    aosBinary: String? = nil, aosHome: String? = nil,
                    expectedAosBinary: String? = nil, launchError: String? = nil,
                    nativeInputConfig: String? = nil, openOverview: Bool = false) {
        let session = TraySession(store: store)
        session.aosBinary = aosBinary
        session.aosHome = aosHome
        session.expectedAosBinary = expectedAosBinary
        if let socketPath {
            do {
                try session.startSocket(path: socketPath)
            } catch let error as SocketEndpointError {
                writeError(error.message)
                exit(1)
            } catch let error as SocketListenerError {
                writeError(error.message)
                exit(1)
            } catch {
                writeError("failed to bind Unix socket")
                exit(1)
            }
        }

        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        let delegate = TrayAppDelegate(session: session, openOverview: openOverview, nativeInputConfig: nativeInputConfig, launchError: launchError)
        application.delegate = delegate
        // NSApplication does not retain its delegate. Keep the connection and
        // window lifetime owner alive in optimized builds as well as previews.
        withExtendedLifetime(delegate) { application.run() }
    }

    private static func writeError(_ message: String) {
        let text = "aos-tray: \(message)\n"
        try? FileHandle.standardError.write(contentsOf: Data(text.utf8))
    }
}

@MainActor
final class TrayAppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSMenuDelegate {
    private let session: TraySession
    private var statusItem: NSStatusItem?
    private var panel: NSWindow?
    private var panelController: NSHostingController<PanelView>?
    private var hadPendingRequests = false
    private var permissionPanel: NSPanel?
    private let openOverview: Bool
    private var nativeInputConfig: String?
    private let launchError: String?
    private var nativeInputStatusItem: NSMenuItem?
    private var nativeSetupWindow: NativeRuntimeSetupWindow?
    private let nativeInput = NativeRuntimeInputService()
    private var updateRefresh: Task<Void, Never>?

    init(session: TraySession, openOverview: Bool = false, nativeInputConfig: String? = nil, launchError: String? = nil) {
        self.session = session
        self.openOverview = openOverview
        self.nativeInputConfig = nativeInputConfig
        self.launchError = launchError
        super.init()
        session.onRuntimePromptsChanged = { [weak self] rows in
            guard let self else { return }
            let shouldReveal = !self.hadPendingRequests && !rows.isEmpty
            self.hadPendingRequests = !rows.isEmpty
            self.refreshStatusTitle()
            if rows.isEmpty { self.permissionPanel?.orderOut(nil) }
            guard shouldReveal else { return }
            self.revealPermission()
        }
        session.onUpdatesChanged = { [weak self] in self?.refreshStatusTitle() }
    }

    private func refreshStatusTitle() {
        let requests = session.runtimePrompts.count
        // Permission requests take priority. No private capsule names appear
        // in the menu bar, and update discovery never opens a modal prompt.
        statusItem?.button?.title = requests > 0 ? "AOS · \(requests)" :
            (session.updates?.items.contains(where: \.canApply) == true ? "AOS ↑" : "AOS")
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installStatusItem()
        installPanel()
        installPermissionPanel()
        updateRefresh = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                await self.session.runUpdates(.refresh)
                // The backend owns the daily freshness policy and shared lock.
                // This wakeup only keeps a long-running tray's badge current.
                do { try await Task.sleep(for: .seconds(14_400)) } catch { return }
            }
        }
        do {
            try LoginItemRegistration.registerInstalledApplication()
        } catch {
            session.lastError = "AOS Command Center could not enable Open at Login. Enable it in System Settings > General > Login Items."
        }
        adoptDefaultNativeInputIfNeeded()
        reconnectNativeInput(nil)
        if openOverview { showOverview(nil) }
        if let launchError {
            session.applyLaunchError(launchError)
        }
        if !session.runtimePrompts.isEmpty {
            revealPermission()
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        session.presentation.lifecycle.terminateAfterLastWindowClosed
    }

    func applicationWillTerminate(_ notification: Notification) {
        updateRefresh?.cancel()
        nativeInput.stop()
        session.stopSocket()
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if session.presentation.lifecycle.closeWindowHides {
            sender.orderOut(nil)
            return false
        }
        return true
    }

    @objc
    func showOverview(_ sender: Any?) {
        session.show(.overview)
        revealPanel()
        Task { await session.refreshOverview() }
    }

    @objc
    func showRequests(_ sender: Any?) {
        if !session.runtimePrompts.isEmpty {
            revealPermission()
            return
        }
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

    @objc
    func showUpdates(_ sender: Any?) {
        session.show(.updates)
        revealPanel()
    }

    @objc
    func chooseNativeInput(_ sender: Any?) {
        let picker = NSOpenPanel()
        picker.title = "Connect Native Input"
        picker.message = "Choose the private connection file for your AOS runtime. This uses an already-paired credential; it does not enroll a service or grant permissions."
        picker.prompt = "Connect"
        picker.canChooseDirectories = false
        picker.allowsMultipleSelection = false
        NSApp.activate(ignoringOtherApps: true)
        guard picker.runModal() == .OK, let url = picker.url else { return }
        // Validate before replacing the existing connection. The file is only
        // selected for this app session; credentials are not copied or displayed.
        do {
            _ = try NativeRuntimeConfiguration.load(path: url.path)
            nativeInputConfig = url.path
            reconnectNativeInput(nil)
        } catch {
            session.lastError = "Choose a valid private native-input connection file owned by your user. The existing connection was not changed."
            revealPanel()
        }
    }

    @objc
    func setupNativeInput(_ sender: Any?) {
        if nativeSetupWindow != nil { return }
        guard session.aosBinary != nil, session.aosHome != nil else {
            let alert = NSAlert()
            alert.messageText = "Native input setup unavailable"
            alert.informativeText = session.store.isDemo
                ? NativeRuntimeSetupCopy.demoUnavailable
                : NativeRuntimeSetupCopy.unavailableMessage(expectedBinary: session.expectedAosBinary)
            alert.alertStyle = .informational
            alert.addButton(withTitle: "OK")
            alert.runModal()
            return
        }
        if NativeRuntimeSetup.existingEnrollment(home: session.aosHome, configPath: nativeInputConfig) {
            let alert = NSAlert()
            alert.messageText = "Native input already enrolled"
            alert.informativeText = NativeRuntimeSetupCopy.existing
            alert.alertStyle = .informational
            alert.addButton(withTitle: "OK")
            alert.runModal()
            return
        }
        let window = NativeRuntimeSetupWindow(session: session) { [weak self] receipt in
            guard let self else { return }
            self.nativeSetupWindow = nil
            guard receipt != nil else { return }
            let alert = NSAlert()
            alert.messageText = NativeRuntimeSetupCopy.restartNeededTitle
            alert.informativeText = NativeRuntimeSetupCopy.restartNeeded
            alert.alertStyle = .informational
            alert.addButton(withTitle: "OK")
            alert.runModal()
        }
        nativeSetupWindow = window
        window.show()
    }

    @objc
    func reconnectNativeInput(_ sender: Any?) {
        guard let nativeInputConfig else { return }
        nativeInputStatusItem?.title = "Native Input: Connecting…"
        nativeInput.start(configPath: nativeInputConfig) { [weak self] error in
            guard let self else { return }
            self.session.lastError = error
            self.nativeInputStatusItem?.title = error == nil
                ? "Native Input: Connected" : "Native Input: Needs Attention"
        }
    }

    private func adoptDefaultNativeInputIfNeeded() {
        guard nativeInputConfig == nil, let home = session.aosHome else { return }
        nativeInputConfig = NativeRuntimeSetup.adoptableConnectionPath(home: home)
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
        let overview = NSMenuItem(title: "Open AOS", action: #selector(showOverview(_:)), keyEquivalent: "")
        overview.target = self
        menu.addItem(overview)
        let inputStatus = NSMenuItem(title: "Native Input: Not Connected", action: nil, keyEquivalent: "")
        inputStatus.isEnabled = false
        nativeInputStatusItem = inputStatus
        menu.addItem(inputStatus)
        let connect = NSMenuItem(title: "Connect Native Input…", action: #selector(chooseNativeInput(_:)), keyEquivalent: "")
        connect.target = self
        menu.addItem(connect)
        let setup = NSMenuItem(title: "Set Up Native Input…", action: #selector(setupNativeInput(_:)), keyEquivalent: "")
        setup.target = self
        menu.addItem(setup)
        let reconnect = NSMenuItem(title: "Reconnect Native Input", action: #selector(reconnectNativeInput(_:)), keyEquivalent: "")
        reconnect.target = self
        menu.addItem(reconnect)
        menu.addItem(.separator())

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
        let updates = NSMenuItem(title: "Updates…", action: #selector(showUpdates(_:)), keyEquivalent: "")
        updates.target = self
        menu.addItem(updates)

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

    private func installPermissionPanel() {
        let window = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 388, height: 390),
            styleMask: [.titled, .closable], backing: .buffered, defer: false
        )
        window.title = "AOS · Permission"
        window.isReleasedWhenClosed = false
        window.hidesOnDeactivate = false
        window.delegate = self
        window.contentViewController = NSHostingController(rootView: PermissionDialog(session: session) { [weak window] in
            window?.orderOut(nil)
        })
        window.center()
        permissionPanel = window
    }

    private func revealPermission() {
        guard let permissionPanel else { return }
        permissionPanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
}
