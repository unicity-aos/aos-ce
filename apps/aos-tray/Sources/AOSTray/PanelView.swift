import SwiftUI
import AOSTrayCore

struct PanelView: View {
    @ObservedObject var session: TraySession
    @State private var capsuleSearch = ""

    var body: some View {
        let presentation = session.presentation
        VStack(alignment: .leading, spacing: 18) {
            if presentation.showsDemoBanner {
                Text(presentation.demoBannerText)
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.black)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.yellow)
                    .accessibilityIdentifier("demo-banner")
                    .accessibilityLabel(presentation.demoBannerText)
            }

            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("AOS").font(.system(size: 24, weight: .semibold))
                    Text(session.aosHome != nil ? "Your local command center" :
                         session.nativeSocket ? "Ready for permission requests" : "Local companion preview")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer()
                Image(systemName: "shield.lefthalf.filled")
                    .font(.system(size: 26, weight: .light))
                    .foregroundStyle(.tint).accessibilityHidden(true)
            }

            Picker("Section", selection: $session.section) {
                Text("Overview").tag(PanelSection.overview)
                Text("Requests").tag(PanelSection.requests)
                Text("Capsules").tag(PanelSection.capsules)
                Text("Updates").tag(PanelSection.updates)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .accessibilityLabel("Panel section")

            if let error = session.lastError {
                Text(error)
                    .font(.system(size: 12))
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("decision-error")
            }

            Group {
                if session.section == .overview {
                    OverviewView(session: session)
                } else if session.section == .requests {
                    requestList(presentation)
                } else if session.section == .updates {
                    UpdatesView(session: session)
                } else if session.aosHome != nil {
                    CapsuleLibraryView(session: session)
                } else {
                    capsuleList(presentation)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            Divider()
            DisclosureGroup("Connection details") {
                Text("Presenter: \(presentation.nativeConnectionLabel)\nInventory: \(presentation.inventoryLabel)\n\(presentation.explanation)")
                    .font(.caption).foregroundStyle(.secondary)
                    .padding(.top, 6)
            }
            .font(.caption).foregroundStyle(.secondary)
        }
        .padding(20)
        .frame(width: 440, height: 520, alignment: .topLeading)
    }

    @ViewBuilder
    private func requestList(_ presentation: TrayPresentation) -> some View {
        if presentation.runtimePrompts.isEmpty && presentation.requests.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Image(systemName: "checkmark.circle")
                    .font(.system(size: 32, weight: .light))
                    .foregroundStyle(.secondary).accessibilityHidden(true)
                Text("No pending requests").font(.headline)
                Text(session.nativeSocket ? "When an agent needs your permission, it will appear here." : "Connect AOS to review requests here.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("empty-requests")
            }
            .padding(.vertical, 40)
            .frame(maxWidth: .infinity)
        } else {
            ScrollView {
              VStack(alignment: .leading, spacing: 16) {
                if !presentation.runtimePrompts.isEmpty {
                    if let prompt = presentation.runtimePrompts.first {
                        runtimePromptRow(prompt)
                    }
                    if presentation.runtimePrompts.count > 1 {
                        Label("\(presentation.runtimePrompts.count - 1) more waiting", systemImage: "tray.full")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                } else if presentation.requests.isEmpty {
                    Text(presentation.emptyRuntimePromptsText)
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("empty-runtime-prompts")
                }
                ForEach(presentation.requests) { request in
                    inventoryRequestRow(request)
                }
              }
              .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    @ViewBuilder
    private func runtimePromptRow(_ prompt: RuntimePromptRow) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Image(systemName: "hand.raised.fill")
                .font(.system(size: 28)).foregroundStyle(.tint)
                .padding(12)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 14))
                .accessibilityHidden(true)
            Text("Your permission is needed")
                .font(.title3.weight(.semibold))
            Text(prompt.message)
                .font(.body).fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            Text("Message from the connected runtime")
                .font(.caption)
                .foregroundStyle(.secondary)
            if let consent = prompt.consent {
                ConsentDetails(consent: consent)
            }
            Divider()
            VStack(spacing: 8) {
                ForEach(Array(prompt.options.enumerated()), id: \.offset) { index, label in
                    Button {
                        session.selectRuntimePrompt(promptID: prompt.id, index: index)
                    } label: {
                        ConsentChoiceLabel(label: label, index: index, consent: prompt.consent)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .accessibilityIdentifier("permission-option-\(index)")
                }
            }
        }
        .padding(18)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.primary.opacity(0.09)))
        .accessibilityIdentifier("runtime-prompt-\(prompt.id)")
    }

    @ViewBuilder
    private func inventoryRequestRow(_ request: RequestRow) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(request.stateLabel)
                .font(.system(size: 11, weight: .semibold))
            Text("principal \(request.principal)")
            Text("capsule \(request.capsule)")
            Text("scope \(request.scope)")
            Text(request.reason)
                .foregroundStyle(.secondary)
            if let applied = request.appliedDecision {
                Text("decision \(applied)")
                    .foregroundStyle(.secondary)
            }
            if request.prompt {
                HStack {
                    ForEach(request.decisions) { button in
                        Button(button.label) {
                            session.apply(requestID: request.id, decision: button.verb)
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.large)
                        .accessibilityLabel(button.label)
                    }
                }
            }
        }
        .textSelection(.enabled)
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(
            "\(request.stateLabel), principal \(request.principal), capsule \(request.capsule), scope \(request.scope)"
        )
    }

    @ViewBuilder
    private func capsuleList(_ presentation: TrayPresentation) -> some View {
        if presentation.capsules.isEmpty {
            VStack(spacing: 12) {
                Image(systemName: "square.stack.3d.up")
                    .font(.system(size: 32, weight: .light)).accessibilityHidden(true)
                Text("Your capsule library").font(.headline)
                Text("Connect an AOS installation to browse the capsules visible to its runtime principal.")
                    .font(.callout).multilineTextAlignment(.center)
            }
            .foregroundStyle(.secondary).padding(.vertical, 40)
            .frame(maxWidth: .infinity)
        } else {
            TextField("Search capsules or agents", text: $capsuleSearch)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("Search capsules or agents")
            List(presentation.capsules.filter { capsule in
                capsuleSearch.isEmpty || capsule.capsule.localizedCaseInsensitiveContains(capsuleSearch)
                    || capsule.principal.localizedCaseInsensitiveContains(capsuleSearch)
            }) { capsule in
                VStack(alignment: .leading, spacing: 6) {
                    Text(capsule.stateLabel)
                        .font(.system(size: 11, weight: .semibold))
                    Text(capsule.capsule).font(.headline)
                    Text(capsule.principal).font(.subheadline).foregroundStyle(.secondary)
                    Text(capsule.scope).font(.callout)
                    if capsule.pregranted {
                        Text("Pregranted. No prompt.")
                            .foregroundStyle(.secondary)
                    }
                }
                .textSelection(.enabled)
                .padding(.vertical, 4)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(
                    "\(capsule.stateLabel), principal \(capsule.principal), capsule \(capsule.capsule), scope \(capsule.scope)"
                )
            }
            .listStyle(.plain)
        }
    }
}
