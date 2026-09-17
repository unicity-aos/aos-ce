import AppKit
import SwiftUI
import AOSTrayCore

/// One native request window. The transport owner retains it and calls cancel
/// on disconnect or expiry; closing the window also cancels, never accepts.
@MainActor
final class NativeInputWindow: NSObject, NSWindowDelegate {
    private let panel: NSPanel
    private var resolution: NativeInputResolution
    private var completion: ((NativeInputAnswer) -> Void)?

    init(request: NativeInputRequest, preview: Bool = false, notice: String? = nil,
         onComplete: @escaping (NativeInputAnswer) -> Void) throws {
        resolution = try NativeInputResolution(request: request)
        completion = onComplete
        panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 340, height: 360),
                        styleMask: [.titled, .closable], backing: .buffered, defer: false)
        super.init()
        panel.title = preview ? "AOS · Input preview" : "AOS · Input requested"
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.delegate = self
        let content = VStack(spacing: 0) {
            if preview {
                Text("Preview · No runtime connected")
                    .font(.caption).foregroundStyle(.secondary).padding(.top, 12)
            }
            if let notice {
                Text(notice).font(.caption).foregroundStyle(.secondary).padding(.top, 12)
            }
            NativeInputDialog(request: request) { [weak self] answer in self?.finish(answer) }
        }
        let controller = NSHostingController(rootView: content)
        panel.contentViewController = controller
        panel.setContentSize(controller.view.fittingSize)
        panel.center()
    }

    func show() {
        guard !resolution.isFinished else { return }
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func cancel() { finish(.cancelled) }

    func windowWillClose(_ notification: Notification) { cancel() }

    private func finish(_ answer: NativeInputAnswer) {
        guard let result = resolution.take(answer) else { return }
        let callback = completion
        completion = nil
        panel.orderOut(nil)
        // Release editor state before notifying the transport. Swift String
        // storage is not guaranteed to be zeroized by releasing this view.
        panel.contentViewController = nil
        panel.close()
        callback?(result)
    }
}
