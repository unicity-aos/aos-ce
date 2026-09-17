import Foundation
import AOSTrayCore

enum OverviewReader {
    /// Explicit executable only; no PATH lookup, shell, init, or daemon start.
    static func read(binary: String, home: String) async throws -> RuntimeOverview {
        try await StatusCommandReader.read(binary: binary, home: home)
    }
}
