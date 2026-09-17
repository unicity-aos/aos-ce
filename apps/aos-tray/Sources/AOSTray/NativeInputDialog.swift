import SwiftUI
import AOSTrayCore

/// Native input form; request transport and window lifetime are supplied externally.
struct NativeInputDialog: View {
    let request: NativeInputRequest
    let onComplete: (NativeInputAnswer) -> Void

    var body: some View {
        NativeInputDialogContent(request: request, onComplete: onComplete)
            .id(request.id)
    }
}

private struct NativeInputDialogContent: View {
    let request: NativeInputRequest
    let onComplete: (NativeInputAnswer) -> Void
    @State private var draft: NativeInputDraft
    @State private var finished = false

    init(request: NativeInputRequest, onComplete: @escaping (NativeInputAnswer) -> Void) {
        self.request = request
        self.onComplete = onComplete
        _draft = State(initialValue: NativeInputDraft(request: request))
    }

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: iconName)
                .font(.system(size: 26, weight: .medium))
                .foregroundStyle(.tint)
                .frame(width: 52, height: 52)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
                .accessibilityHidden(true)
            ScrollView {
                Text(request.prompt)
                    .font(request.prompt.count > 120 ? .body : .headline)
                    .multilineTextAlignment(request.prompt.count > 120 ? .leading : .center)
                    .frame(maxWidth: .infinity)
                    .textSelection(.enabled)
            }.frame(maxHeight: 160).fixedSize(horizontal: false, vertical: true)
            VStack(spacing: 4) {
                Text("Capsule: \(request.capsule)")
                    .font(.subheadline)
                    .textSelection(.enabled)
                Text("Principal: \(request.principal)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            .multilineTextAlignment(.center)
            Group {
                switch request.kind {
                case .text:
                    TextField("Value", text: $draft.text)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit(submit)
                        .accessibilityIdentifier("native-input-text")
                case .secret:
                    SecureField("Secret", text: $draft.secret)
                        .textFieldStyle(.roundedBorder)
                        .privacySensitive()
                        .onSubmit(submit)
                        .accessibilityIdentifier("native-input-secret")
                case .select:
                    Picker("Choice", selection: $draft.selection) {
                        ForEach(request.options ?? [], id: \.self) { option in
                            Text(option).tag(option)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.menu)
                    .accessibilityIdentifier("native-input-select")
                case .array:
                    arrayEditor
                }
            }
            HStack(spacing: 10) {
                Button("Cancel", action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .frame(maxWidth: .infinity)
                Button("Continue", action: submit)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(!draft.canSubmit(request))
                    .frame(maxWidth: .infinity)
            }
            .controlSize(.large)
        }
        .padding(24)
        .frame(width: 340)
        .fixedSize(horizontal: false, vertical: true)
        .onDisappear(perform: clearSecrets)
    }

    @ViewBuilder
    private var arrayEditor: some View {
        VStack(alignment: .leading, spacing: 8) {
            ScrollView {
                VStack(spacing: 6) {
                    ForEach($draft.rows) { $row in
                        HStack(spacing: 6) {
                            TextField("Item", text: $row.value)
                                .textFieldStyle(.roundedBorder)
                            Button {
                                draft.removeRow(id: row.id)
                            } label: {
                                Image(systemName: "minus.circle")
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Remove item")
                        }
                    }
                }
            }
            .frame(maxHeight: 160)
            Button("Add item") { draft.addRow() }
                .disabled(draft.rows.count >= NativeInputRequest.maxItems)
                .accessibilityIdentifier("native-input-array-add")
        }
        .accessibilityIdentifier("native-input-array")
    }

    private var iconName: String {
        switch request.kind {
        case .text: "character.cursor.ibeam"
        case .secret: "key.fill"
        case .select: "switch.2"
        case .array: "list.bullet.rectangle"
        }
    }

    private func cancel() {
        finish(.cancelled)
    }

    private func submit() {
        finish(draft.answer(for: request))
    }

    private func finish(_ answer: NativeInputAnswer) {
        guard !finished else { return }
        do {
            try request.validateAnswer(answer)
        } catch {
            return
        }
        finished = true
        clearSecrets()
        onComplete(answer)
    }

    private func clearSecrets() {
        draft.clearSecrets()
    }
}
