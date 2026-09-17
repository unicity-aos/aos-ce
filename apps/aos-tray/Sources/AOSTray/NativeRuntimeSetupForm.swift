import AppKit
import SwiftUI
import AOSTrayCore

struct NativeRuntimeSetupForm: View {
    let discovery: PrincipalDiscovery?
    let selected: String?
    let confirmEnroll: Bool
    let confirmRoute: Bool
    let busy: Bool
    let message: String?
    let select: @MainActor @Sendable (String) -> Void
    let setEnroll: @MainActor @Sendable (Bool) -> Void
    let setRoute: @MainActor @Sendable (Bool) -> Void
    let cancel: @MainActor @Sendable () -> Void
    let confirm: @MainActor @Sendable () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Set Up Native Input").font(.headline)
            Text(NativeRuntimeSetupCopy.localPersonalExplanation)
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            content
            Toggle(NativeRuntimeSetupCopy.enrollConfirm, isOn: enrollBinding)
                .disabled(busy || !canConfirm)
            Toggle(NativeRuntimeSetupCopy.routeConfirm, isOn: routeBinding)
                .disabled(busy || !canConfirm)
            if let message {
                Text(message).font(.callout).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Button("Cancel", action: cancel).disabled(busy)
                Spacer()
                Button("Set Up") { confirm() }
                    .disabled(busy || !readyToSetUp)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(16)
        .frame(width: 360)
        .disabled(busy)
    }

    private var enrollBinding: Binding<Bool> {
        Binding(get: { confirmEnroll }, set: { setEnroll($0) })
    }

    private var routeBinding: Binding<Bool> {
        Binding(get: { confirmRoute }, set: { setRoute($0) })
    }

    private var canConfirm: Bool {
        if case .owned(let principals) = discovery {
            return principals.contains { $0.enabled }
        }
        return false
    }

    private var readyToSetUp: Bool {
        if case .run = NativeRuntimeSetup.evaluate(
            selected: selected, discovery: discovery ?? .failed,
            confirmEnroll: confirmEnroll, confirmRoute: confirmRoute
        ) {
            return true
        }
        return false
    }

    @ViewBuilder
    private var content: some View {
        switch discovery {
        case .owned(let principals) where principals.isEmpty:
            Text(PrincipalDiscoveryCopy.emptyDirectory).font(.callout)
        case .owned(let principals):
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(principals) { principal in
                        Button {
                            if principal.enabled { select(principal.id) }
                        } label: {
                            HStack {
                                Text(principal.id)
                                Spacer()
                                if !principal.enabled {
                                    Text("Unavailable").font(.caption).foregroundStyle(.secondary)
                                } else if principal.id == selected {
                                    Image(systemName: "checkmark")
                                }
                            }
                        }
                        .buttonStyle(.plain)
                        .disabled(!principal.enabled || busy)
                        .accessibilityLabel(principal.enabled ? principal.id : "\(principal.id), unavailable")
                    }
                }
            }.frame(maxHeight: 160)
        case .unsupported:
            Text(NativeRuntimeSetupCopy.unsupported).font(.callout)
        case .failed:
            Text(PrincipalDiscoveryCopy.failed).font(.callout)
        case nil:
            Text("Discovering owned principals…").font(.callout).foregroundStyle(.secondary)
        }
    }
}

@MainActor
final class NativeRuntimeSetupWindow: NSObject, NSWindowDelegate {
    private let panel: NSPanel
    private let session: TraySession
    private var selected: String?
    private var confirmEnroll = false
    private var confirmRoute = false
    private var busy = false
    private var message: String?
    private var discovery: PrincipalDiscovery?
    private var onFinished: ((NativeRuntimeSetupReceipt?) -> Void)?

    init(session: TraySession, onFinished: @escaping (NativeRuntimeSetupReceipt?) -> Void) {
        self.session = session
        self.onFinished = onFinished
        panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 360, height: 420),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        super.init()
        panel.title = "AOS · Native Input Setup"
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.delegate = self
        render()
    }

    func show() {
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        Task { await refreshDiscovery() }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        !busy
    }

    func windowWillClose(_ notification: Notification) {
        finish(nil)
    }

    private func refreshDiscovery() async {
        guard let binary = session.aosBinary, let home = session.aosHome else {
            discovery = .failed
            message = NativeRuntimeSetupCopy.unavailableMessage(expectedBinary: session.expectedAosBinary)
            render()
            return
        }
        do {
            discovery = try await PrincipalDiscoveryReader.read(binary: binary, home: home)
        } catch {
            discovery = .failed
        }
        if discovery == .unsupported {
            message = NativeRuntimeSetupCopy.unsupported
        } else if discovery == .failed {
            message = PrincipalDiscoveryCopy.failed
        } else if case .owned(let principals) = discovery, principals.isEmpty {
            message = PrincipalDiscoveryCopy.emptyDirectory
        }
        render()
    }

    private func confirm() {
        guard !busy else { return }
        let action = NativeRuntimeSetup.evaluate(
            selected: selected, discovery: discovery ?? .failed,
            confirmEnroll: confirmEnroll, confirmRoute: confirmRoute
        )
        guard case .run(let principal) = action else {
            message = NativeRuntimeSetupCopy.message(for: action)
            render()
            return
        }
        guard let binary = session.aosBinary, let home = session.aosHome else {
            message = NativeRuntimeSetupCopy.unavailableMessage(expectedBinary: session.expectedAosBinary)
            render()
            return
        }
        busy = true
        message = nil
        render()
        Task {
            do {
                let receipt = try await NativeRuntimeSetup.run(
                    binary: binary, home: home, principal: principal,
                    confirmEnroll: true, confirmRoute: true
                )
                finish(receipt)
            } catch let error as NativeRuntimeSetupError {
                busy = false
                message = NativeRuntimeSetupCopy.message(for: error)
                render()
            } catch {
                busy = false
                message = NativeRuntimeSetupCopy.failed
                render()
            }
        }
    }

    private func render() {
        let controller = NSHostingController(rootView: NativeRuntimeSetupForm(
            discovery: discovery,
            selected: selected,
            confirmEnroll: confirmEnroll,
            confirmRoute: confirmRoute,
            busy: busy,
            message: message,
            select: { [weak self] principal in
                self?.selected = principal
                self?.message = nil
                self?.render()
            },
            setEnroll: { [weak self] value in
                self?.confirmEnroll = value
                self?.render()
            },
            setRoute: { [weak self] value in
                self?.confirmRoute = value
                self?.render()
            },
            cancel: { [weak self] in self?.finish(nil) },
            confirm: { [weak self] in self?.confirm() }
        ))
        panel.contentViewController = controller
        panel.setContentSize(controller.view.fittingSize)
    }

    private func finish(_ receipt: NativeRuntimeSetupReceipt?) {
        let callback = onFinished
        onFinished = nil
        panel.delegate = nil
        panel.orderOut(nil)
        panel.contentViewController = nil
        panel.close()
        callback?(receipt)
    }
}
