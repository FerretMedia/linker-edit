import LinkerEditKit
import SwiftUI

/// Choosing the edit computer: its Tailscale address, a recent one, or one linked to your Linker account. Grouped
/// panels on the system's grouped background; the buttons are glass.
struct ConnectView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("lastTypedHost") private var address = ""
    @State private var computers: [LinkedComputer]?
    @State private var linkerBusy = false
    @State private var linkerProblem: String?
    @State private var signedIn = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                header
                if let problem = model.problem {
                    Label(problem, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.orange)
                        .accessibilityIdentifier("connect-problem")
                }
                addressCard
                if !model.recent.isEmpty { recentCard }
                linkerCard
            }
            .frame(maxWidth: 560)
            .padding(.horizontal, 24)
            .padding(.vertical, 48)
            .frame(maxWidth: .infinity)
        }
        .background(Color(.systemGroupedBackground))
        .onAppear { signedIn = model.linker.isSignedIn }
    }

    private var header: some View {
        HStack(spacing: 14) {
            Image(systemName: "film.stack")
                .font(.system(size: 26, weight: .semibold))
                .foregroundStyle(.tint)
                .frame(width: 52, height: 52)
                .background(Color.accentColor.opacity(0.15), in: RoundedRectangle(cornerRadius: 14))
            VStack(alignment: .leading, spacing: 2) {
                Text("Linker-Edit").font(.title2.weight(.semibold))
                Text("Drive the agent on your edit computer from this iPad.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var addressCard: some View {
        Card(title: "Edit computer") {
            Text("Its remote access address from Linker-Edit's settings, such as https://studio.your-tailnet.ts.net:8443. This iPad needs Tailscale, logged in as the computer's owner.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            HStack(spacing: 10) {
                TextField("https://studio.your-tailnet.ts.net:8443", text: $address)
                    .textContentType(.URL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.go)
                    .onSubmit(connect)
                    .padding(.horizontal, 14)
                    .frame(height: 48)
                    .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 12))
                    .accessibilityIdentifier("host-field")
                Button("Connect", action: connect)
                    .buttonStyle(.glassProminent)
                    .controlSize(.large)
                    .disabled(address.trimmingCharacters(in: .whitespaces).isEmpty)
                    .accessibilityIdentifier("connect-button")
            }
        }
    }

    private func connect() {
        model.connect(typed: address)
    }

    private var recentCard: some View {
        Card(title: "Recent") {
            ForEach(model.recent) { host in
                HStack {
                    Button {
                        Task { await model.open(host) }
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: host.via == .linker ? "link" : "network")
                                .foregroundStyle(.secondary)
                                .frame(width: 20)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(host.name ?? host.endpoint.displayName).foregroundStyle(.primary)
                                Text(host.via == .linker ? "Linker Connect · \(host.endpoint.displayName)" : host.endpoint.displayName)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Button(role: .destructive) {
                        model.forget(host)
                    } label: {
                        Image(systemName: "xmark").font(.caption)
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Forget \(host.endpoint.displayName)")
                }
                .padding(.vertical, 4)
            }
        }
    }

    private var linkerCard: some View {
        Card(title: "Linker Connect") {
            Text("No Tailscale? Sign in with the Linker account your edit computer is linked to, and open it from anywhere.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            if !model.linker.isConfigured {
                Text("Signing in with Linker isn't set up in this build yet.")
                    .font(.footnote)
                    .foregroundStyle(.orange)
            } else if !signedIn {
                Button {
                    Task { await signIn() }
                } label: {
                    Label("Sign in with Linker", systemImage: "person.crop.circle")
                }
                .buttonStyle(.glass)
                .disabled(linkerBusy)
            } else {
                computerList
                HStack {
                    Button("Refresh") { Task { await loadComputers() } }
                    Spacer()
                    Button("Sign out", role: .destructive) {
                        model.linker.signOut()
                        signedIn = false
                        computers = nil
                    }
                }
                .buttonStyle(.borderless)
                .font(.footnote)
            }
            if linkerBusy { ProgressView().controlSize(.small) }
            if let linkerProblem {
                Text(linkerProblem).font(.footnote).foregroundStyle(.red)
            }
        }
        .task(id: signedIn) {
            if signedIn && computers == nil { await loadComputers() }
        }
    }

    @ViewBuilder
    private var computerList: some View {
        if let computers, computers.isEmpty {
            Text("No computers are linked to this Linker account. Turn on Linker Connect in Linker-Edit's settings on the computer.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        ForEach(computers ?? []) { computer in
            Button {
                guard let endpoint = computer.endpoint else { return }
                Task { await model.openWithLinker(endpoint, name: computer.name) }
            } label: {
                HStack(spacing: 12) {
                    Circle()
                        .fill(computer.online == true ? Color.green : computer.online == false ? Color.red : Color.secondary)
                        .frame(width: 8, height: 8)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(computer.name).foregroundStyle(.primary)
                        Text(computer.online == false ? "Offline" : computer.endpoint?.displayName ?? computer.url)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.vertical, 4)
        }
    }

    private func signIn() async {
        linkerBusy = true
        linkerProblem = nil
        defer { linkerBusy = false }
        do {
            try await model.linker.signIn()
            signedIn = true
        } catch ConnectError.cancelled {
            return
        } catch {
            linkerProblem = error.localizedDescription
        }
    }

    private func loadComputers() async {
        linkerBusy = true
        linkerProblem = nil
        defer { linkerBusy = false }
        do {
            computers = try await model.linker.computers()
        } catch {
            signedIn = model.linker.isSignedIn
            linkerProblem = error.localizedDescription
        }
    }
}

/// A rounded panel with a small title, like a grouped list section.
struct Card<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title.uppercased())
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 20))
    }
}
