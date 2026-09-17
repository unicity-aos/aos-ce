import Foundation

public struct MenuItemPresentation: Equatable, Codable, Sendable {
    public var title: String
    public var accessibilityLabel: String
    public var section: PanelSection?
    public var quitsTray: Bool
}

public struct CapsuleRow: Equatable, Codable, Sendable, Identifiable {
    public var id: String {
        InventoryIdentity.capsule(PrincipalID(principal), CapsuleID(capsule), Scope(scope))
    }
    public var principal: String
    public var capsule: String
    public var scope: String
    public var state: GrantDisplayState
    public var stateLabel: String
    public var pregranted: Bool
    public var prompt: Bool
}

public struct DecisionButton: Equatable, Codable, Sendable, Identifiable {
    public var id: String { verb.rawValue }
    public var verb: DecisionVerb
    public var label: String
}

public struct RequestRow: Equatable, Codable, Sendable, Identifiable {
    public var id: String
    public var principal: String
    public var capsule: String
    public var scope: String
    public var reason: String
    public var state: GrantDisplayState
    public var stateLabel: String
    public var appliedDecision: String?
    public var decisions: [DecisionButton]
    public var prompt: Bool
}

public struct TrayPresentation: Equatable, Codable, Sendable {
    public var productName: String
    public var bundleIdentifier: String
    public var statusItemTitle: String
    public var statusItemAccessibility: String
    public var connection: RuntimeConnection
    public var connectionLabel: String
    public var nativeConnectionLabel: String
    public var inventoryLabel: String
    public var explanation: String
    public var isDemo: Bool
    public var showsDemoBanner: Bool
    public var demoBannerText: String
    public var runtimeSuppliedCaption: String
    public var runtimePrompts: [RuntimePromptRow]
    public var capsules: [CapsuleRow]
    public var requests: [RequestRow]
    public var menu: [MenuItemPresentation]
    public var lifecycle: TrayLifecyclePolicy
    public var emptyRequestsText: String
    public var emptyCapsulesText: String
    public var emptyRuntimePromptsText: String

    public static func make(
        from store: TrayStore,
        nativeSocket: Bool = false,
        runtimePrompts: [RuntimePromptRow] = []
    ) -> TrayPresentation {
        let connectionLabel = ProductIdentity.disconnectedLabel
        let nativeConnectionLabel = nativeSocket
            ? ProductIdentity.nativeConnectionLocalSocket
            : ProductIdentity.nativeConnectionOff
        let statusAccessibility: String
        if store.isDemo {
            statusAccessibility = "\(ProductIdentity.name), \(connectionLabel), demo fixture"
        } else if nativeSocket {
            statusAccessibility = "\(ProductIdentity.name), \(connectionLabel), local socket"
        } else {
            statusAccessibility = "\(ProductIdentity.name), \(connectionLabel)"
        }
        let explanation: String
        if store.isDemo {
            explanation = ProductIdentity.demoExplanation
        } else if nativeSocket {
            explanation = ProductIdentity.socketExplanation
        } else {
            explanation = ProductIdentity.disconnectedExplanation
        }

        return TrayPresentation(
            productName: ProductIdentity.name,
            bundleIdentifier: ProductIdentity.bundleIdentifier,
            statusItemTitle: ProductIdentity.statusItemTitle,
            statusItemAccessibility: statusAccessibility,
            connection: store.connection,
            connectionLabel: connectionLabel,
            nativeConnectionLabel: nativeConnectionLabel,
            inventoryLabel: ProductIdentity.inventoryUnavailable,
            explanation: explanation,
            isDemo: store.isDemo,
            showsDemoBanner: store.isDemo,
            demoBannerText: store.isDemo ? ProductIdentity.demoBanner : "",
            runtimeSuppliedCaption: ProductIdentity.runtimeSuppliedCaption,
            runtimePrompts: runtimePrompts,
            capsules: store.capsules.map { capsule in
                CapsuleRow(
                    principal: capsule.principal.value,
                    capsule: capsule.capsule.value,
                    scope: capsule.scope.value,
                    state: capsule.state,
                    stateLabel: capsule.pregranted
                        ? "Pregranted"
                        : capsule.state.label,
                    pregranted: capsule.pregranted,
                    prompt: capsule.prompt
                )
            },
            requests: store.requests.map { request in
                RequestRow(
                    id: request.id,
                    principal: request.principal.value,
                    capsule: request.capsule.value,
                    scope: request.scope.value,
                    reason: request.reason,
                    state: request.state,
                    stateLabel: request.state.label,
                    appliedDecision: request.appliedDecision?.rawValue,
                    decisions: request.visibleDecisions.map {
                        DecisionButton(verb: $0, label: $0.label)
                    },
                    prompt: request.prompt
                )
            },
            menu: [
                MenuItemPresentation(
                    title: ProductIdentity.requestsMenuTitle,
                    accessibilityLabel: "Open Requests",
                    section: .requests,
                    quitsTray: false
                ),
                MenuItemPresentation(
                    title: ProductIdentity.capsulesMenuTitle,
                    accessibilityLabel: "Open Capsules",
                    section: .capsules,
                    quitsTray: false
                ),
                MenuItemPresentation(
                    title: ProductIdentity.quitMenuTitle,
                    accessibilityLabel: ProductIdentity.quitAccessibility,
                    section: nil,
                    quitsTray: true
                ),
            ],
            lifecycle: store.lifecycle,
            emptyRequestsText: store.isDemo
                ? "No fixture requests."
                : "No requests. Runtime is DISCONNECTED.",
            emptyCapsulesText: store.isDemo
                ? "No fixture capsules."
                : "No capsules. Runtime is DISCONNECTED.",
            emptyRuntimePromptsText: ProductIdentity.emptyRuntimePromptsText
        )
    }

    public func json() throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        let data = try encoder.encode(self)
        guard let text = String(data: data, encoding: .utf8) else {
            throw TrayPresentationEncodingError.invalidUTF8
        }
        return text.hasSuffix("\n") ? text : text + "\n"
    }
}

public enum TrayPresentationEncodingError: Error {
    case invalidUTF8
}
