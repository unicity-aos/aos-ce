#if DEBUG
import AppKit
import CryptoKit
import AOSTrayCore

/// Uses only the disposable Rust example's public synthetic credentials.
/// Not compiled into the release app and not a production enrollment path.
@MainActor
enum NativeRuntimeInputPreview {
    static func run(path: String) {
        guard path.hasPrefix("/private/tmp/ani-"), path.hasSuffix("/fixture-runtime/run/system.sock") else { return }
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let delegate = RuntimeInputPreviewDelegate(path: path)
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}

@MainActor
private final class RuntimeInputPreviewDelegate: NSObject, NSApplicationDelegate {
    let path: String
    var presenter: NativeInputPresenter?
    var connection: NativeRuntimeInputConnection?
    var task: Task<Void, Never>?
    var completed = false

    init(path: String) { self.path = path }

    func applicationDidFinishLaunching(_ notification: Notification) {
        task = Task { [self] in
            do {
                let presenter = try NativeInputPresenter(capacity: 1,
                    notice: "Local test runtime · Synthetic input only")
                self.presenter = presenter
                let socket = try await NativeRuntimeSocket.connect(path: path, timeout: 2)
                let key = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(repeating: 7, count: 32))
                try await socket.authenticate(principal: "native-input-test", token: Data(repeating: 0xab, count: 32),
                                              signingKey: key, timeout: 2)
                let connection = try NativeRuntimeInputConnection(socket: socket, principal: "native-input-test",
                    capacity: 1, inputTimeout: .seconds(120), ioTimeout: 2,
                    collect: { request, owner, timeout in
                        await presenter.collect(request, connection: owner, timeout: timeout)
                    }, dismiss: { presenter.disconnect($0) }, delivered: { [weak self] _, status in
                        self?.completed = status == .delivered
                        print(status == .delivered ? "input-delivered" : "input-rejected")
                        self?.connection?.close()
                        NSApp.terminate(nil)
                    })
                self.connection = connection
                try await connection.run(readTimeout: 120)
            } catch {
                if !completed { print("input-disconnected") }
                NSApp.terminate(nil)
            }
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        connection?.close()
        presenter?.cancelAll()
        task?.cancel()
    }
}
#endif
