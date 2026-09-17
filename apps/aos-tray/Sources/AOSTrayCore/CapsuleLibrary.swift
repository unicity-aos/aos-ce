import Foundation

/// Principal-visible metadata; not a grants list or a global installation count.
public struct CapsuleLibrary: Decodable, Equatable, Sendable {
    public enum State: String, Decodable, Sendable { case available, stopped, unavailable }
    public let principal: String
    public let state: State
    public let capsules: [Entry]?

    public struct Entry: Decodable, Equatable, Sendable, Identifiable {
        public var id: String { name }
        public let name: String
        public let version: String
        public let description: String?
    }

    func validate() throws {
        guard !principal.isEmpty,
              (state == .available) == (capsules != nil) else { throw OverviewError.invalidStatus }
        if let capsules {
            guard capsules.allSatisfy({ !$0.name.isEmpty }),
                  Set(capsules.map(\.name)).count == capsules.count else { throw OverviewError.invalidStatus }
        }
    }

    public func matching(_ query: String) -> [Entry] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return (capsules ?? []).filter {
            query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)
                || ($0.description?.localizedCaseInsensitiveContains(query) ?? false)
        }
    }
}
