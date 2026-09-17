import Foundation

/// Retry only transport unavailability. Invalid credentials, frames and
/// configuration require operator attention rather than repeated attempts.
@MainActor
public enum NativeRuntimeReconnectLoop {
    public static func run(
        delay: Duration,
        attempt: () async throws -> Void,
        interrupted: () -> Void,
        sleep: (Duration) async throws -> Void = { try await Task.sleep(for: $0) }
    ) async throws {
        guard delay > .zero else { throw NativeInputError.invalidRequest }
        while true {
            try Task.checkCancellation()
            do {
                try await attempt()
                return
            } catch NativeRuntimeSocketError.unavailable {
                try Task.checkCancellation()
                interrupted()
                try await sleep(delay)
            }
        }
    }
}
