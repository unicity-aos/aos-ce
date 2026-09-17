import Foundation
import AOSTrayCore

/// Runtime adapters retain this presenter and disconnect their session on EOF.
/// This does not register an MCP responder or write a secret to configuration.
@MainActor
final class NativeInputPresenter {
    private var window: NativeInputWindow?
    private var windowTicket: UUID?
    private var coordinator: NativeInputCoordinator!
    private let notice: String?

    init(capacity: Int, notice: String? = nil) throws {
        self.notice = notice
        coordinator = try NativeInputCoordinator(capacity: capacity,
            present: { [weak self] ticket, request in self?.show(ticket, request) },
            dismiss: { [weak self] ticket in self?.dismiss(ticket) })
    }

    func collect(_ request: NativeInputRequest, connection: UUID,
                 timeout: Duration) async -> NativeInputAnswer {
        await coordinator.collect(request, connection: connection, timeout: timeout)
    }

    func disconnect(_ connection: UUID) { coordinator.disconnect(connection) }
    func cancelAll() { coordinator.cancelAll() }

    /// The caller owns authentication and socket lifetime; disconnect the
    /// returned session on EOF so every pending native window is cancelled.
    func runtimeSession(principal: String,
                        exchange: @escaping NativeRuntimeInputSession.Exchange) -> NativeRuntimeInputSession {
        NativeRuntimeInputSession(principal: principal,
            collect: { [weak self] request, connection, timeout in
                guard let self else { return .cancelled }
                return await self.collect(request, connection: connection, timeout: timeout)
            }, dismiss: { [weak self] connection in self?.disconnect(connection) }, exchange: exchange)
    }

    private func show(_ ticket: UUID, _ request: NativeInputRequest) {
        do {
            let window = try NativeInputWindow(request: request, notice: notice) { [weak self] answer in
                self?.coordinator.submit(ticket: ticket, answer: answer)
            }
            self.window = window
            windowTicket = ticket
            window.show()
        } catch {
            coordinator.submit(ticket: ticket, answer: .cancelled)
        }
    }

    private func dismiss(_ ticket: UUID) {
        guard windowTicket == ticket else { return }
        let old = window
        window = nil
        windowTicket = nil
        old?.cancel()
    }
}
