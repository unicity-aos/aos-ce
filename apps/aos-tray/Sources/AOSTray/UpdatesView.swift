import SwiftUI
import AOSTrayCore

struct UpdatesView: View {
    @ObservedObject var session: TraySession
    @State private var channel = "stable"
    @State private var selection: String?
    @State private var choosingPrincipal = false
    private var channelMatches: Bool { session.updates?.channel == channel }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Software updates").font(.headline)
                Spacer()
                if session.updatesBusy { ProgressView().controlSize(.small) }
            }
            HStack {
                Picker("Channel", selection: $channel) {
                    Text("Stable").tag("stable")
                    Text("Dev").tag("dev")
                    Text("Nightly").tag("nightly")
                }.labelsHidden().frame(width: 110)
                Spacer()
                Button(session.updatesError == nil ? "Check for Updates" : "Retry") {
                    Task { await session.runUpdates(.check(channel: channel)) }
                }.disabled(session.updatesBusy)
            }
            if let error = session.updatesError {
                Text(error).font(.callout).foregroundStyle(.red)
            }
            if !channelMatches {
                Text("Check the selected channel to see its updates.")
                    .font(.callout).foregroundStyle(.secondary)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach((channelMatches ? (session.updates?.items ?? []) : []) + (session.capsuleUpdates?.items ?? [])) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Image(systemName: "shippingbox").foregroundStyle(.secondary)
                                Text(item.name).font(.headline)
                                Spacer()
                                if item.canApply {
                                    Button("Update") { selection = item.id }
                                        .disabled(session.updatesBusy)
                                }
                            }
                            Text(item.candidateVersion.flatMap { $0 == item.installedVersion ? nil : "\(item.installedVersion) → \($0)" } ?? item.installedVersion)
                                .font(.caption).foregroundStyle(.secondary)
                            Text(item.label).font(.subheadline.weight(.medium))
                            Text(item.message).font(.caption).foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                                .textSelection(.enabled)
                        }
                        Divider()
                    }
                }
            }
            HStack {
                Button("Capsules: \(session.libraryPrincipal)…") {
                    choosingPrincipal = true
                    Task { await session.refreshLibrary() }
                }.popover(isPresented: $choosingPrincipal) {
                    LibraryPrincipalForm(discovery: session.principalDiscovery,
                        selected: session.libraryPrincipal,
                        cancel: { choosingPrincipal = false }, select: { principal in
                            choosingPrincipal = false
                            Task { await session.selectLibraryPrincipal(principal) }
                        })
                }
                Spacer()
                Button("Check capsules") {
                    Task { await session.runUpdates(.capsules(principal: session.libraryPrincipal)) }
                }
            }.font(.caption).disabled(session.updatesBusy || session.refreshingLibrary)
            Text("Only this user’s owned principals are shown. Checking never starts the runtime or grants permissions.")
                .font(.caption2).foregroundStyle(.secondary)
            if channelMatches, let checked = session.updates?.checkedAt {
                Text("Last checked \(Date(timeIntervalSince1970: TimeInterval(checked)), style: .relative) ago")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if channelMatches && session.updates?.items.contains(where: \.canApply) == true {
                Button("Update All") { selection = "all" }
                    .buttonStyle(.borderedProminent).disabled(session.updatesBusy)
            }
        }
        .task {
            await session.runUpdates(.list)
            channel = session.updates?.channel ?? "stable"
        }
        .onChange(of: channel) { _ in selection = nil }
        .confirmationDialog("Install selected updates?", isPresented: Binding(
            get: { selection != nil }, set: { if !$0 { selection = nil } }
        ), titleVisibility: .visible) {
            Button("Install Updates") {
                if let chosen = selection { Task { await session.runUpdates(.apply(selection: chosen, channel: channel)) } }
                selection = nil
            }
            Button("Cancel", role: .cancel) { selection = nil }
        } message: {
            Text("Packages will be verified before installation. Active runtime and coding sessions may need to reconnect. No broader capsule permissions are granted.")
        }
    }
}
