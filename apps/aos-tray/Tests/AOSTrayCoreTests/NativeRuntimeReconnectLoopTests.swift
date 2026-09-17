import Foundation
import Testing
@testable import AOSTrayCore

@MainActor
struct NativeRuntimeReconnectLoopTests {
    @Test func transportFailureRetriesWithFreshAttempt() async throws {
        var attempts = 0
        var interruptions = 0
        var sleeps: [Duration] = []
        try await NativeRuntimeReconnectLoop.run(delay: .seconds(5), attempt: {
            attempts += 1
            if attempts == 1 { throw NativeRuntimeSocketError.unavailable }
        }, interrupted: { interruptions += 1 }, sleep: { sleeps.append($0) })
        #expect(attempts == 2)
        #expect(interruptions == 1)
        #expect(sleeps == [.seconds(5)])
    }

    @Test func authenticationAndProtocolFailuresNeverRetry() async {
        for error in [NativeRuntimeSocketError.authenticationFailed, .invalidFrame] {
            var attempts = 0
            do {
                try await NativeRuntimeReconnectLoop.run(delay: .seconds(1), attempt: {
                    attempts += 1
                    throw error
                }, interrupted: { Issue.record("Permanent failure was retried") },
                sleep: { _ in Issue.record("Permanent failure slept") })
                Issue.record("Permanent failure was hidden")
            } catch {}
            #expect(attempts == 1)
        }
    }

    @Test func cancellationDuringDelayStopsRetries() async {
        var attempts = 0
        do {
            try await NativeRuntimeReconnectLoop.run(delay: .seconds(1), attempt: {
                attempts += 1
                throw NativeRuntimeSocketError.unavailable
            }, interrupted: {}, sleep: { _ in throw CancellationError() })
            Issue.record("Cancellation was hidden")
        } catch { #expect(error is CancellationError) }
        #expect(attempts == 1)
    }
}
