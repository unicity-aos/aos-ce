import Foundation
import Testing
@testable import AOSTrayCore

@Suite
struct LaunchArgumentTests {
    @Test func defaultIsDisconnected() throws {
        let parsed = try LaunchArguments.parse(["aos-tray"]).get()
        #expect(parsed.mode == .disconnected)
        #expect(parsed.snapshot == false)
        #expect(parsed.help == false)
        #expect(parsed.socketPath == nil)
        #expect(parsed.aosBinary == nil)
        #expect(parsed.aosHome == nil)
    }

    @Test func socketPathIsExclusive() {
        let parsed = try? LaunchArguments.parse(["aos-tray", "--socket", "/private/tmp/aos.sock"]).get()
        #expect(parsed?.socketPath == "/private/tmp/aos.sock")
        #expect(parsed?.mode == .disconnected)
        #expect(LaunchArguments.parse(["aos-tray", "--socket"]) == .failure(.missingSocketPath))
        #expect(LaunchArguments.parse(["aos-tray", "--socket", "--help"]) == .failure(.missingSocketPath))
        #expect(LaunchArguments.parse(["aos-tray", "--demo", "--socket", "/x"]) == .failure(.conflictingModes))
        #expect(LaunchArguments.parse(["aos-tray", "--socket", "/x", "--demo"]) == .failure(.conflictingModes))
        #expect(LaunchArguments.parse(["aos-tray", "--snapshot", "--socket", "/x"]) == .failure(.conflictingModes))
        #expect(LaunchArguments.parse(["aos-tray", "--socket", "/x", "--snapshot"]) == .failure(.conflictingModes))
    }

    @Test func demoAndSnapshot() throws {
        let parsed = try LaunchArguments.parse(["aos-tray", "--demo", "--snapshot"]).get()
        #expect(parsed.mode == .demo)
        #expect(parsed.snapshot)
    }

    @Test func unknownArgumentFails() {
        let parsed = LaunchArguments.parse(["aos-tray", "--connect"])
        #expect(parsed == .failure(.unknownArgument("--connect")))
    }
}

@Suite
struct DisconnectedPresentationTests {
    @Test func emptyInventoryAndDisconnectedLabel() throws {
        let presentation = TrayPresentation.make(from: .disconnected())
        #expect(presentation.connection == .disconnected)
        #expect(presentation.connectionLabel == "DISCONNECTED")
        #expect(presentation.productName == "AOS")
        #expect(presentation.capsules.isEmpty)
        #expect(presentation.requests.isEmpty)
        #expect(!presentation.isDemo)
        #expect(!presentation.showsDemoBanner)
        #expect(presentation.nativeConnectionLabel == "OFF")
        #expect(presentation.inventoryLabel == "UNAVAILABLE")
        #expect(presentation.runtimePrompts.isEmpty)
        #expect(presentation.emptyRequestsText.contains("DISCONNECTED"))
        #expect(presentation.emptyCapsulesText.contains("DISCONNECTED"))
        let json = try presentation.json()
        #expect(json.contains("DISCONNECTED"))
        #expect(!json.lowercased().contains("password"))
        #expect(!json.lowercased().contains("api_key"))
        #expect(!json.lowercased().contains("secret"))
    }

    @Test func applyIsRefusedWhileDisconnected() {
        var store = TrayStore.disconnected()
        let result = store.applyDecision(requestID: "req-http-1", decision: .deny)
        #expect(result == .failure(.disconnected))
        #expect(store.requests.isEmpty)
        #expect(store.connection == .disconnected)
    }

    @Test func lifecycleDoesNotTouchRuntime() {
        let policy = TrayLifecyclePolicy.accessoryShell
        #expect(policy.activationPolicy == .accessory)
        #expect(!policy.showsDockIcon)
        #expect(policy.closeWindowHides)
        #expect(!policy.terminateAfterLastWindowClosed)
        #expect(!policy.quitStopsRuntime)
        #expect(!policy.autoLaunchAtLogin)
        #expect(!policy.readsLiveHome)
        #expect(!policy.collectsCredentials)
        #expect(!policy.performsNetworking)
        #expect(!policy.installsApps)
        #expect(!policy.startsDaemons)
    }
}

@Suite
struct DemoFixtureTests {
    @Test func demoStaysDisconnectedWithBanner() {
        let presentation = TrayPresentation.make(from: .demo())
        #expect(presentation.connection == .disconnected)
        #expect(presentation.connectionLabel == "DISCONNECTED")
        #expect(presentation.isDemo)
        #expect(presentation.showsDemoBanner)
        #expect(presentation.demoBannerText.contains("DEMO FIXTURE"))
        #expect(!presentation.capsules.isEmpty)
        #expect(!presentation.requests.isEmpty)
        #expect(presentation.nativeConnectionLabel == "OFF")
        #expect(presentation.inventoryLabel == "UNAVAILABLE")
        #expect(presentation.runtimePrompts.isEmpty)
    }

    @Test func pregrantedCapsulesDoNotPrompt() {
        let presentation = TrayPresentation.make(from: .demo())
        let pregranted = presentation.capsules.filter(\.pregranted)
        #expect(!pregranted.isEmpty)
        #expect(pregranted.allSatisfy { $0.prompt == false })
        #expect(pregranted.allSatisfy { $0.stateLabel == "Pregranted" })
        #expect(!presentation.requests.contains { $0.capsule == "aos-fs" && $0.scope == "fs.read" })
        #expect(!presentation.requests.contains { $0.capsule == "aos-shell" })
    }

    @Test func requestsKeepDistinctIdsForTheSameScope() {
        let store = TrayStore.demo()
        let http = store.requests.filter {
            $0.principal.value == "demo-principal"
                && $0.capsule.value == "aos-http"
                && $0.scope.value == "http.fetch"
        }
        #expect(http.map(\.id) == ["req-http-1", "req-http-2"])
        let presentation = TrayPresentation.make(from: store)
        #expect(presentation.requests.filter { $0.capsule == "aos-http" }.map(\.id) == ["req-http-1", "req-http-2"])
    }

    @Test func decisionAppliesToOneRequestIdOnly() {
        var store = TrayStore.demo()
        let result = store.applyDecision(requestID: "req-http-1", decision: .approveOnce)
        #expect(result.isSuccess)
        let first = store.requests.first { $0.id == "req-http-1" }
        let second = store.requests.first { $0.id == "req-http-2" }
        #expect(first?.state == .approved)
        #expect(first?.appliedDecision == .approveOnce)
        #expect(second?.state == .pending)
        #expect(second?.appliedDecision == nil)
        #expect(store.connection == .disconnected)
        #expect(store.isDemo)
        let presentation = TrayPresentation.make(from: store)
        #expect(presentation.showsDemoBanner)
        #expect(presentation.requests.first { $0.id == "req-http-1" }?.decisions.isEmpty == true)
        #expect(presentation.requests.first { $0.id == "req-http-2" }?.decisions.map(\.verb) == [.approveOnce, .deny])
    }

    @Test func unsupportedAlwaysIsRejected() {
        var store = TrayStore.demo()
        let result = store.applyDecision(requestID: "req-http-1", decision: .approveAlways)
        #expect(result == .failure(.unsupportedDecision(.approveAlways)))
        #expect(store.requests.first { $0.id == "req-http-1" }?.state == .pending)
    }

    @Test func httpRequestDoesNotOfferAlways() {
        let row = TrayPresentation.make(from: .demo()).requests.first { $0.id == "req-http-1" }
        #expect(row?.decisions.map(\.verb) == [.approveOnce, .deny])
        #expect(row?.decisions.map(\.label) == ["Approve Once", "Deny"])
        #expect(!(row?.decisions.map(\.verb).contains(.approveAlways) ?? true))
    }

    @Test func memoryRequestOffersHostVerbsWithoutAlways() {
        let row = TrayPresentation.make(from: .demo()).requests.first { $0.id == "req-memory-1" }
        #expect(row?.decisions.map(\.verb) == [.approve, .approveSession, .deny])
        #expect(!(row?.decisions.map(\.verb).contains(.approveAlways) ?? true))
    }

    @Test func nonPendingRequestsHaveNoDecisionButtons() {
        let presentation = TrayPresentation.make(from: .demo())
        for id in ["req-skills-1", "req-session-1", "req-mcp-1"] {
            let row = presentation.requests.first { $0.id == id }
            #expect(row != nil)
            #expect(row?.prompt == false)
            #expect(row?.decisions.isEmpty == true)
        }
        #expect(presentation.requests.first { $0.id == "req-skills-1" }?.state == .denied)
        #expect(presentation.requests.first { $0.id == "req-session-1" }?.state == .expired)
        #expect(presentation.requests.first { $0.id == "req-mcp-1" }?.state == .unavailable)
    }

    @Test func expiredAndUnavailableRejectDecisions() {
        var store = TrayStore.demo()
        #expect(store.applyDecision(requestID: "req-session-1", decision: .deny) == .failure(.notActionable(.expired)))
        #expect(store.applyDecision(requestID: "req-mcp-1", decision: .approve) == .failure(.notActionable(.unavailable)))
    }

    @Test func capsuleIdentityIncludesPrincipalAndScope() {
        let store = TrayStore.demo()
        let readDemo = store.capsules.first {
            $0.principal.value == "demo-principal" && $0.capsule.value == "aos-fs" && $0.scope.value == "fs.read"
        }
        let readOther = store.capsules.first {
            $0.principal.value == "other-principal" && $0.capsule.value == "aos-fs" && $0.scope.value == "fs.read"
        }
        let writeDemo = store.capsules.first {
            $0.principal.value == "demo-principal" && $0.capsule.value == "aos-fs" && $0.scope.value == "fs.write"
        }
        #expect(readDemo != nil)
        #expect(readOther != nil)
        #expect(writeDemo != nil)
        #expect(readDemo?.id != readOther?.id)
        #expect(readDemo?.id != writeDemo?.id)
        #expect(readOther?.id != writeDemo?.id)
        let ids = store.capsules.map(\.id)
        #expect(Set(ids).count == ids.count)
        let presentation = TrayPresentation.make(from: store)
        #expect(Set(presentation.capsules.map(\.id)).count == presentation.capsules.count)
    }

    @Test func customSameScopeRequestsAreNotDropped() {
        let fixture = DemoFixture(
            capsules: [],
            requests: [
                PermissionRequest(
                    id: "a",
                    principal: PrincipalID("p"),
                    capsule: CapsuleID("c"),
                    scope: Scope("s"),
                    reason: "one",
                    state: .pending,
                    supportedDecisions: [.approveOnce, .deny]
                ),
                PermissionRequest(
                    id: "b",
                    principal: PrincipalID("p"),
                    capsule: CapsuleID("c"),
                    scope: Scope("s"),
                    reason: "two",
                    state: .pending,
                    supportedDecisions: [.deny]
                ),
            ]
        )
        var store = TrayStore.demo(fixture)
        #expect(store.requests.map(\.id) == ["a", "b"])
        #expect(store.applyDecision(requestID: "b", decision: .deny).isSuccess)
        #expect(store.requests.first { $0.id == "a" }?.state == .pending)
        #expect(store.requests.first { $0.id == "b" }?.state == .denied)
        #expect(store.applyDecision(requestID: "a", decision: .approveOnce).isSuccess)
        #expect(store.requests.first { $0.id == "a" }?.state == .approved)
        #expect(store.requests.first { $0.id == "b" }?.state == .denied)
    }
}

@Suite
struct NativeSocketPresentationTests {
    @Test func socketPresentationKeepsInventoryDisconnected() throws {
        let prompt = RuntimePromptRow(
            id: "11111111-1111-1111-1111-111111111111",
            requestID: "correlation-id",
            message: "Runtime-supplied explanation",
            options: ["Allow once", "Deny this"]
        )
        let presentation = TrayPresentation.make(
            from: .disconnected(),
            nativeSocket: true,
            runtimePrompts: [prompt]
        )
        #expect(presentation.connection == .disconnected)
        #expect(presentation.connectionLabel == "DISCONNECTED")
        #expect(presentation.nativeConnectionLabel == "LOCAL SOCKET")
        #expect(presentation.inventoryLabel == "UNAVAILABLE")
        #expect(presentation.capsules.isEmpty)
        #expect(presentation.requests.isEmpty)
        #expect(presentation.runtimePrompts.map(\.options) == [["Allow once", "Deny this"]])
        #expect(presentation.runtimeSuppliedCaption == "Runtime-supplied message")
        #expect(presentation.explanation.contains("not human authenticity proof"))
        let json = try presentation.json()
        #expect(json.contains("LOCAL SOCKET"))
        #expect(json.contains("UNAVAILABLE"))
        #expect(json.contains("Allow once"))
        #expect(json.contains("Deny this"))
        #expect(!json.contains("principal"))
        #expect(!json.lowercased().contains("password"))
        #expect(!json.lowercased().contains("secret"))
    }
}

private extension Result where Success == PermissionRequest, Failure == TrayError {
    var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
}
