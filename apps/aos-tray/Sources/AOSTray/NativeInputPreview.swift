#if DEBUG
import AppKit
import SwiftUI
import AOSTrayCore

/// Offline visual fixture. Never starts a runtime, opens a socket, or persists answers.
@MainActor
enum NativeInputPreview {
    static func run(kind: String) {
        let prompts = [
            "text": "What should we call your workspace?",
            "secret": "Enter your GitHub access token",
            "select": "Which repository should this capsule use?",
            "array": "Which repositories should this capsule watch?",
            "long": String(repeating: "This is a longer capsule request to check that every part of the explanation stays readable. ", count: 12),
        ]
        guard let prompt = prompts[kind] else { return }
        var fixture: [String: Any] = [
            "id": UUID().uuidString, "principal": "codex-code",
            "capsule": "github-demo", "key": "demo_input", "prompt": prompt,
            "kind": kind == "long" ? "text" : kind,
        ]
        if kind == "select" { fixture["options"] = ["astrid-runtime/astrid", "unicity-aos/aos-ce"] }
        guard let bytes = try? JSONSerialization.data(withJSONObject: fixture),
              let request = try? NativeInputRequest.decode(bytes) else { return }
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        let delegate = InputPreviewDelegate(request: request)
        application.delegate = delegate
        withExtendedLifetime(delegate) { application.run() }
    }
}

@MainActor
private final class InputPreviewDelegate: NSObject, NSApplicationDelegate {
    let request: NativeInputRequest
    var window: NativeInputWindow?

    init(request: NativeInputRequest) { self.request = request }

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = try? NativeInputWindow(request: request, preview: true) { _ in
            // Deliberately do not serialize or log even synthetic answers.
            NSApp.terminate(nil)
        }
        guard let window else { NSApp.terminate(nil); return }
        window.show()
    }
}
#endif
