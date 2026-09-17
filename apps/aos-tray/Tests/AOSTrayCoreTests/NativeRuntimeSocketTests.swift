import Foundation
import Testing
@testable import AOSTrayCore

@Suite struct NativeRuntimeSocketTests {
    private struct CancelResponder: PromptResponder {
        func decide(_ request: ValidatedPresentationRequest) async -> Int? { nil }
    }

    @Test func refusesRegularFilesAndSymlinkEndpoints() async throws {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("not-socket")
        try Data().write(to: file)
        let link = root.appendingPathComponent("alias")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: file)
        for path in [file.path, link.path, root.path + "/../not-socket"] {
            do {
                let socket = try await NativeRuntimeSocket.connect(path: path, timeout: 1)
                socket.close()
                Issue.record("invalid endpoint accepted")
            } catch { #expect(String(describing: error) == "unavailable") }
        }
    }

    @Test(arguments: [false, true]) func interruptedReadClosesConnection(cancel: Bool) async throws {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let path = root.appendingPathComponent("peer.sock").path
        // This peer deliberately waits for a presentation request and sends
        // nothing. It tests interruption, not the Astrid wire protocol.
        let server = UnixPresentationServer(path: path, responder: CancelResponder())
        try server.start()
        defer { server.stop() }
        let socket = try await NativeRuntimeSocket.connect(path: path, timeout: 1)
        defer { socket.close() }
        let read = Task { try await socket.readFrame(timeout: cancel ? 5 : 0.05) }
        if cancel {
            try await Task.sleep(for: .milliseconds(20))
            read.cancel()
        }
        do { _ = try await read.value; Issue.record("interrupted read succeeded") }
        catch {}
        do {
            try await socket.writeFrame(Data([1]), timeout: 1)
            Issue.record("partially consumed or cancelled connection reused")
        } catch {}
    }

    private func directory() throws -> URL {
        let root = URL(fileURLWithPath: "/private/tmp/ans-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        return root
    }
}
