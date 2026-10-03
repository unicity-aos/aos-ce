import AppKit
import Combine
import AOSTrayCore

@MainActor
final class TraySession: ObservableObject {
    @Published private(set) var store: TrayStore
    @Published var section: PanelSection
    @Published var lastError: String?
    @Published private(set) var runtimePrompts: [RuntimePromptRow] = []
    @Published private(set) var overview: RuntimeOverview?
    @Published private(set) var volume: VolumeFileInfo?
    @Published private(set) var refreshingOverview = false
    @Published private(set) var overviewError: String?
    @Published private(set) var library: CapsuleLibrary?
    @Published private(set) var refreshingLibrary = false
    @Published private(set) var libraryError: String?
    @Published private(set) var updates: UpdateInventory?
    @Published private(set) var updatesBusy = false
    @Published private(set) var updatesError: String?
    private(set) var updateRetry: UpdateCommand?
    @Published private(set) var capsuleUpdates: UpdateInventory?
    @Published private(set) var libraryPrincipal = "default"
    @Published private(set) var principalDiscovery: PrincipalDiscovery?
    @Published private(set) var checkingMount = false
    @Published private(set) var filesBusy = false
    @Published private(set) var mountError: String?
    var aosBinary: String?
    var aosHome: String?
    var expectedAosBinary: String?
    var volumeBusy: Bool { checkingMount || filesBusy }
    var onUpdatesChanged: (() -> Void)?

    func runUpdates(_ command: UpdateCommand) async {
        guard !updatesBusy, let aosBinary, let aosHome else { return }
        updatesBusy = true
        updatesError = nil
        updateRetry = nil
        defer { updatesBusy = false }
        do {
            let result = try await UpdateCommandReader.run(binary: aosBinary, home: aosHome, command: command)
            if case .capsules = command { capsuleUpdates = result } else { updates = result }
            onUpdatesChanged?()
        }
        catch {
            updateRetry = command.retryCommand
            updatesError = error.localizedDescription
        }
    }

    func selectLibraryPrincipal(_ input: String) async {
        guard !refreshingLibrary else { return }
        let principal = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard OwnedPrincipal.isValidID(principal), principal != "anonymous" else { return }
        libraryPrincipal = principal
        capsuleUpdates = nil
        await refreshLibrary()
    }

    func refreshLibrary() async {
        guard !refreshingLibrary, let aosBinary, let aosHome else { return }
        refreshingLibrary = true
        defer { refreshingLibrary = false }
        library = nil
        libraryError = nil
        let discovery: PrincipalDiscovery
        do {
            discovery = try await PrincipalDiscoveryReader.read(binary: aosBinary, home: aosHome)
        } catch {
            principalDiscovery = .failed
            libraryError = PrincipalDiscoveryCopy.failed
            return
        }
        principalDiscovery = discovery
        let choice = PrincipalPicker.choose(selected: libraryPrincipal, from: discovery)
        if case .selected(let principal) = choice {
            libraryPrincipal = principal.id
        } else {
            libraryError = PrincipalDiscoveryCopy.message(for: choice, discovery: discovery)
            return
        }
        do {
            let status = try await StatusCommandReader.read(binary: aosBinary, home: aosHome,
                includeCapsules: true, principal: libraryPrincipal)
            guard let inventory = status.capsuleInventory else { throw OverviewError.invalidStatus }
            library = inventory
        } catch {
            libraryError = "Library unavailable. Check the runtime connection and that this AOS version supports capsule inventory."
        }
    }

    func refreshOverview() async {
        guard !refreshingOverview, let aosBinary, let aosHome else { return }
        refreshingOverview = true
        defer { refreshingOverview = false }
        overview = nil
        overviewError = nil
        volume = try? VolumeFileInfo.read(aosHome: URL(fileURLWithPath: aosHome))
        do { overview = try await OverviewReader.read(binary: aosBinary, home: aosHome) }
        catch { overviewError = "Runtime status is unavailable. Refresh to try again." }
    }

    func revealVolume() {
        guard let aosHome, let current = try? VolumeFileInfo.read(aosHome: URL(fileURLWithPath: aosHome)) else { return }
        NSWorkspace.shared.activateFileViewerSelecting([current.url])
    }

    func openFiles() {
        guard !volumeBusy, aosBinary != nil, aosHome != nil else { return }
        Task { await openFilesNow() }
    }

    func ejectVolume() {
        guard !volumeBusy, aosBinary != nil, aosHome != nil else { return }
        Task { await ejectVolumeNow() }
    }

    func chooseMountedVolume() {
        guard !volumeBusy, aosBinary != nil, aosHome != nil else { return }
        let panel = NSOpenPanel()
        panel.title = "Open mounted AOS volume"
        panel.message = "Choose the mounted volume itself. AOS will verify it belongs to this runtime."
        panel.prompt = "Open volume"
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.directoryURL = URL(fileURLWithPath: "/Volumes")
        panel.begin { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            Task { @MainActor in await self?.openMountedVolume(url) }
        }
    }

    private func openFilesNow() async {
        guard !volumeBusy, let aosBinary, let aosHome else { return }
        filesBusy = true
        mountError = nil
        defer { filesBusy = false }
        do {
            let status = try await StatusCommandReader.read(binary: aosBinary, home: aosHome)
            overview = status
            guard status.state == .running else { throw CommandCenterVolumeError.runtimeStopped }
            let discovery: PrincipalDiscovery
            do {
                discovery = try await PrincipalDiscoveryReader.read(binary: aosBinary, home: aosHome)
            } catch {
                throw CommandCenterVolumeError.invalidPrincipal
            }
            principalDiscovery = discovery
            let url = try await CommandCenterVolume.openFiles(
                binary: aosBinary, home: aosHome, selectedPrincipal: libraryPrincipal,
                discovery: discovery, runtimeState: status.state)
            guard NSWorkspace.shared.open(url) else { throw CommandCenterVolumeError.unverified }
        } catch let error as CommandCenterVolumeError {
            mountError = CommandCenterVolumeCopy.message(for: error)
        } catch {
            mountError = CommandCenterVolumeCopy.commandFailed
        }
    }

    private func ejectVolumeNow() async {
        guard !volumeBusy, let aosBinary, let aosHome else { return }
        filesBusy = true
        mountError = nil
        defer { filesBusy = false }
        do {
            let discovery: PrincipalDiscovery
            do {
                discovery = try await PrincipalDiscoveryReader.read(binary: aosBinary, home: aosHome)
            } catch {
                throw CommandCenterVolumeError.invalidPrincipal
            }
            principalDiscovery = discovery
            try await CommandCenterVolume.eject(
                binary: aosBinary, home: aosHome, selectedPrincipal: libraryPrincipal,
                discovery: discovery)
        } catch let error as CommandCenterVolumeError {
            mountError = CommandCenterVolumeCopy.message(for: error, ejecting: true)
        } catch {
            mountError = CommandCenterVolumeCopy.ejectFailed
        }
    }

    private func openMountedVolume(_ selected: URL) async {
        guard !volumeBusy, let aosBinary, let aosHome else { return }
        checkingMount = true
        mountError = nil
        defer { checkingMount = false }
        do {
            let status = try await StatusCommandReader.read(binary: aosBinary, home: aosHome,
                principal: "default", mountpoint: selected.path)
            guard let mount = status.mountedVolume else { throw OverviewError.invalidStatus }
            let url = try mount.verifiedNativeRoot()
            guard NSWorkspace.shared.open(url) else { throw OverviewError.invalidStatus }
        } catch {
            mountError = overview?.state == .stopped
                ? "Runtime is stopped. Start it before opening a mounted volume. Nothing was mounted or started."
                : "Couldn’t verify this mounted volume. Select its root and check the runtime connection. Nothing was mounted or started."
        }
    }

    private let broker = RuntimePromptBroker()
    private var promptUpdates = RuntimePromptUpdates()
    private var server: UnixPresentationServer?
    private(set) var nativeSocket = false
    var onRuntimePromptsChanged: (([RuntimePromptRow]) -> Void)?

    init(store: TrayStore, section: PanelSection = .requests) {
        self.store = store
        self.section = section
        broker.onChange = { [weak self] snapshot in
            Task { @MainActor in
                guard let self, self.promptUpdates.accept(snapshot) else { return }
                self.runtimePrompts = snapshot.rows
                self.onRuntimePromptsChanged?(snapshot.rows)
            }
        }
    }

    var presentation: TrayPresentation {
        TrayPresentation.make(
            from: store,
            nativeSocket: nativeSocket,
            runtimePrompts: runtimePrompts
        )
    }

    func startSocket(path: String) throws {
        let server = UnixPresentationServer(path: path, responder: broker)
        try server.start()
        self.server = server
        nativeSocket = true
    }

    func stopSocket() {
        broker.cancelAll()
        server?.stop()
        server = nil
        nativeSocket = false
        _ = promptUpdates.accept(broker.currentSnapshot())
        runtimePrompts = []
        onRuntimePromptsChanged?([])
    }

    func applyLaunchError(_ message: String?) {
        lastError = message
        overviewError = message
    }

    func show(_ section: PanelSection) {
        self.section = section
        lastError = nil
    }

    func apply(requestID: String, decision: DecisionVerb) {
        var next = store
        switch next.applyDecision(requestID: requestID, decision: decision) {
        case .success:
            store = next
            lastError = nil
        case .failure(let error):
            lastError = error.message
        }
    }

    func selectRuntimePrompt(promptID: String, index: Int) {
        broker.select(promptID: promptID, index: index)
    }
}
