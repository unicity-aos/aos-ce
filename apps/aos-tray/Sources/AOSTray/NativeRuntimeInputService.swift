import AOSTrayCore
import Foundation

/// Lifetime owner for an explicitly configured production connection.
/// Reauthenticates after transport failure; never replays interrupted answers.
/// Does not enroll keys or start a daemon.
@MainActor
final class NativeRuntimeInputService {
    private var task: Task<Void, Never>?
    private var connection: NativeRuntimeInputConnection?
    private var presenter: NativeInputPresenter?
    private var generation = UUID()

    func start(configPath: String, report: @escaping (String?) -> Void) {
        stop()
        let generation = self.generation
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let config = try NativeRuntimeConfiguration.load(path: configPath)
                try await NativeRuntimeReconnectLoop.run(delay: .seconds(config.ioTimeoutSeconds), attempt: {
                    guard self.generation == generation else { throw CancellationError() }
                    try await self.runConnection(config: config, generation: generation, report: report)
                }, interrupted: {
                    if self.generation == generation {
                        report("Native input is reconnecting. Interrupted requests were cancelled; no answer will be replayed.")
                    }
                })
            } catch {
                if !Task.isCancelled, self.generation == generation {
                    report("Native input is disconnected. Check the selected runtime and paired tray credential.")
                }
            }
            guard self.generation == generation else { return }
            connection?.close()
            presenter?.cancelAll()
            connection = nil
            presenter = nil
        }
    }

    private func runConnection(config: NativeRuntimeConfiguration, generation: UUID,
                               report: @escaping (String?) -> Void) async throws {
        let presenter = try NativeInputPresenter(capacity: config.capacity)
        self.presenter = presenter
        defer {
            presenter.cancelAll()
            if self.generation == generation {
                connection = nil
                self.presenter = nil
            }
        }
        let socket = try await config.connect()
        defer { socket.close() }
        guard !Task.isCancelled, self.generation == generation else { throw CancellationError() }
        let connection = try NativeRuntimeInputConnection(socket: socket, principal: config.principal,
            capacity: config.capacity, inputTimeout: .seconds(config.inputTimeoutSeconds),
            ioTimeout: config.ioTimeoutSeconds,
            collect: { request, owner, timeout in
                await presenter.collect(request, connection: owner, timeout: timeout)
            }, dismiss: { presenter.disconnect($0) }, delivered: { _, status in
                if self.generation == generation, status != .delivered {
                    report("The runtime could not accept this input. No answer was retried.")
                }
            })
        defer { connection.close() }
        self.connection = connection
        report(nil)
        try await connection.run(readTimeout: config.readTimeoutSeconds)
    }

    func stop() {
        generation = UUID()
        task?.cancel()
        task = nil
        connection?.close()
        connection = nil
        presenter?.cancelAll()
        presenter = nil
    }
}
