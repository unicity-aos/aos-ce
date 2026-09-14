import AppKit
import Combine
import AOSTrayCore

@MainActor
final class TraySession: ObservableObject {
    @Published private(set) var store: TrayStore
    @Published var section: PanelSection
    @Published var lastError: String?

    init(store: TrayStore, section: PanelSection = .requests) {
        self.store = store
        self.section = section
    }

    var presentation: TrayPresentation {
        TrayPresentation.make(from: store)
    }

    func show(_ section: PanelSection) {
        self.section = section
        lastError = nil
    }

    func apply(requestID: String, decision: DecisionVerb) {
        var next = store
        switch next.applyDecision(requestID: requestID, decision: decision) {
        case .success:
            store = next
            lastError = nil
        case .failure(let error):
            lastError = error.message
        }
    }
}
