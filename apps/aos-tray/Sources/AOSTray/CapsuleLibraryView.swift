import SwiftUI
import AOSTrayCore

struct CapsuleLibraryView: View {
    @ObservedObject var session: TraySession
    @State private var search = ""
    @State private var choosingPrincipal = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("Your capsule library", systemImage: "square.stack.3d.up").font(.headline)
                Spacer()
                if session.refreshingLibrary { ProgressView().controlSize(.small) }
                Button("Refresh") { Task { await session.refreshLibrary() } }
                    .disabled(session.refreshingLibrary)
            }
            HStack {
                Text("Viewing: \(session.libraryPrincipal)")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
                Button("Change…") {
                    choosingPrincipal = true
                }
                .font(.caption).disabled(session.refreshingLibrary)
                .accessibilityLabel("Change library principal")
                .popover(isPresented: $choosingPrincipal, arrowEdge: .leading) {
                    LibraryPrincipalForm(discovery: session.principalDiscovery,
                        selected: session.libraryPrincipal,
                        cancel: { choosingPrincipal = false }, select: choosePrincipal)
                }
            }
            if let library = session.library {
                switch library.state {
                case .stopped:
                    empty("Runtime is stopped", "The library is available while AOS is running. Opening this screen does not start it.")
                case .unavailable:
                    empty("Library unavailable", "The runtime could not provide this principal’s capsule inventory. No installation or permission changes were made.")
                case .available:
                    TextField("Search capsules", text: $search)
                        .textFieldStyle(.roundedBorder).accessibilityLabel("Search capsules")
                    let entries = library.matching(search)
                    if entries.isEmpty {
                        empty(search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                              ? "No visible capsules" : "No matches",
                              "This list is scoped to the runtime principal shown above.")
                    } else {
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 0) {
                                ForEach(entries) { entry in
                                    VStack(alignment: .leading, spacing: 5) {
                                        HStack(alignment: .firstTextBaseline) {
                                            Text(entry.name).font(.headline)
                                            Spacer()
                                            if !entry.version.isEmpty {
                                                Text(entry.version).font(.caption).foregroundStyle(.secondary)
                                            }
                                        }
                                        if let description = entry.description, !description.isEmpty {
                                            Text(description).font(.callout).foregroundStyle(.secondary).lineLimit(3)
                                        }
                                    }
                                    .textSelection(.enabled).padding(.vertical, 10)
                                    Divider()
                                }
                            }
                        }
                    }
                    Text("Package information, not a list of granted permissions.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            } else if !session.refreshingLibrary {
                empty("Library unavailable", session.libraryError ?? "Refresh to read the capsule library.")
            }
        }
        .task { await session.refreshLibrary() }
    }

    private func choosePrincipal(_ principal: String) {
        choosingPrincipal = false
        search = ""
        Task { await session.selectLibraryPrincipal(principal) }
    }

    private func empty(_ title: String, _ detail: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.headline)
            Text(detail).font(.callout).foregroundStyle(.secondary)
        }.padding(.vertical, 24)
    }
}
