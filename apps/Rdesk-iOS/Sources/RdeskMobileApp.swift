import SwiftUI

@main
struct RdeskMobileApp: App {
    var body: some Scene { WindowGroup { HomeView() } }
}

struct HomeView: View {
    @AppStorage("gatewayHost") private var host = ""
    @AppStorage("gatewayPort") private var port = "9534"
    @StateObject private var session = RemoteSession()
    @StateObject private var discovery = LANDiscovery()
    @Environment(\.scenePhase) private var scenePhase
    @State private var showRemote = false
    @State private var error: String?
    private let accent = Color(red: 0.25, green: 0.42, blue: 0.95)

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    HStack(spacing: 16) {
                        Image(systemName: "desktopcomputer.and.arrow.down")
                            .font(.system(size: 30)).foregroundStyle(accent)
                            .frame(width: 64, height: 64)
                            .background(accent.opacity(0.1), in: RoundedRectangle(cornerRadius: 18))
                        VStack(alignment: .leading, spacing: 5) {
                            Text("让桌面，随手可达").font(.title2.bold())
                            Text("从 iPhone 与 iPad 连接你的电脑")
                                .font(.subheadline).foregroundStyle(.secondary)
                        }
                    }.padding(.top, 8)

                    VStack(alignment: .leading, spacing: 18) {
                        Label("连接电脑", systemImage: "link").font(.headline)
                        VStack(alignment: .leading, spacing: 8) {
                            Text("电脑的局域网地址").font(.subheadline).foregroundStyle(.secondary)
                            TextField("192.168.1.100", text: $host)
                                .keyboardType(.decimalPad).textInputAutocapitalization(.never)
                                .autocorrectionDisabled().textFieldStyle(.roundedBorder)
                                .accessibilityIdentifier("hostField")
                        }
                        HStack {
                            Text("网关端口").foregroundStyle(.secondary)
                            Spacer()
                            TextField("9534", text: $port).keyboardType(.numberPad)
                                .textFieldStyle(.roundedBorder).frame(width: 100)
                                .accessibilityIdentifier("portField")
                        }
                        Button { connect() } label: {
                            Label("连接电脑屏幕", systemImage: "arrow.up.right")
                                .font(.headline).frame(maxWidth: .infinity).padding(.vertical, 7)
                        }.buttonStyle(.borderedProminent).accessibilityIdentifier("connectButton")
                        if let error { Text(error).font(.footnote).foregroundStyle(.red).accessibilityIdentifier("connectionError") }
                    }.card()

                    VStack(alignment: .leading, spacing: 16) {
                        HStack {
                            Label("附近的电脑", systemImage: "wifi").font(.headline)
                            Spacer()
                            if discovery.scanning { ProgressView() }
                            Button(discovery.scanning ? "停止" : "搜索") {
                                if discovery.scanning { discovery.stop() } else { discovery.start(knownHost: host) }
                            }.accessibilityIdentifier("scanButton")
                        }
                        ForEach(discovery.gateways) { gateway in
                            Button {
                                host = gateway.endpoint.host
                                port = String(gateway.endpoint.port)
                                connect()
                            } label: {
                                HStack {
                                    Image(systemName: "desktopcomputer").font(.title2)
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(gateway.name).font(.headline)
                                        Text(gateway.id).font(.caption).foregroundStyle(.secondary)
                                    }
                                    Spacer()
                                    Image(systemName: "chevron.right")
                                }.padding(.vertical, 6)
                            }.buttonStyle(.plain)
                        }
                        Text(discovery.message).font(.footnote).foregroundStyle(.secondary)
                    }.card()

                    VStack(alignment: .leading, spacing: 12) {
                        Label("开始之前", systemImage: "info.circle").font(.headline)
                        Text("1. 在 Windows 电脑上启动 Rdesk 移动网关。\n2. 手机和电脑连接同一局域网。\n3. 允许 Rdesk 访问本地网络，然后搜索或输入地址。")
                            .font(.subheadline).foregroundStyle(.secondary).lineSpacing(6)
                        Text("当前版本使用 JPEG 画面，支持点击、拖动、滚轮和文字输入。仅在受信局域网使用：网关采用明文连接，无配对密码。")
                            .font(.footnote).foregroundStyle(.secondary)
                    }.card()
                    Text("Rdesk for iOS · 0.1.1").font(.caption).foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity)
                }.padding(20).frame(maxWidth: 680).frame(maxWidth: .infinity)
            }
            .background(Color(uiColor: .systemGroupedBackground))
            .navigationTitle("Rdesk")
            .toolbar { ToolbarItemGroup(placement: .keyboard) { Spacer(); Button("完成") { hideKeyboard() } } }
            .fullScreenCover(isPresented: $showRemote) { RemoteView(session: session) { connect() } }
            .onChange(of: showRemote) { shown in if !shown { session.disconnect() } }
            .onChange(of: scenePhase) { phase in
                if phase == .background {
                    discovery.stop()
                    if session.active { session.disconnect(message: "应用已进入后台，连接已安全断开。") }
                }
            }
            .task {
                #if DEBUG
                // Simulator UI integration tests only; shipping builds ignore these.
                if ProcessInfo.processInfo.arguments.contains("--gateway-fixture") {
                    host = "127.0.0.1"
                    port = "19534"
                    connect()
                }
                #endif
            }
        }.tint(accent)
    }

    private func connect() {
        do {
            let endpoint = try GatewayEndpoint(host: host, port: port)
            host = endpoint.host
            port = String(endpoint.port)
            error = nil
            discovery.stop()
            hideKeyboard()
            session.connect(endpoint)
            showRemote = true
        } catch { self.error = error.localizedDescription }
    }
}

private struct RemoteView: View {
    @ObservedObject var session: RemoteSession
    let reconnect: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var keyboard = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Circle().fill(session.canControl ? .green : .orange).frame(width: 8, height: 8)
                VStack(alignment: .leading, spacing: 3) {
                    Text(session.message).font(.subheadline.bold()).lineLimit(3)
                        .accessibilityIdentifier("sessionStatus")
                    if session.canControl {
                        Text("\(session.dimensions) · \(session.fps) FPS · JPEG").font(.caption).foregroundStyle(.secondary)
                            .accessibilityIdentifier("streamStats")
                    }
                }
                Spacer(minLength: 0)
                Button("断开") { session.disconnect(); dismiss() }
                    .buttonStyle(.bordered).accessibilityIdentifier("disconnectButton")
            }.padding(14)

            ZStack {
                Color.black
                RemoteCanvas(image: session.image, enabled: session.canControl, pointer: session.pointer)
                if session.image == nil {
                    VStack(spacing: 16) {
                        if session.active { ProgressView().tint(.white) }
                        else { Image(systemName: "wifi.exclamationmark").font(.largeTitle) }
                        Text(session.message).multilineTextAlignment(.center)
                        if !session.active {
                            Button("重新连接", action: reconnect).buttonStyle(.borderedProminent)
                                .accessibilityIdentifier("reconnectButton")
                        }
                    }.foregroundStyle(.white).padding(24)
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity).clipped()

            if keyboard {
                HStack {
                    TextField("发送到电脑的文字", text: $text).textFieldStyle(.roundedBorder)
                        .accessibilityIdentifier("remoteTextField")
                    Button("发送") { if session.sendText(text) { text = "" } }
                        .disabled(!session.canControl || text.isEmpty).accessibilityIdentifier("sendTextButton")
                }.padding(.horizontal).padding(.top, 10)
                if let error = session.inputError { Text(error).font(.caption).foregroundStyle(.red) }
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 12) {
                    Button { keyboard.toggle() } label: { Label("键盘", systemImage: "keyboard") }
                        .accessibilityIdentifier("keyboardButton")
                    Button("回车") { session.key(13) }.accessibilityIdentifier("enterButton")
                    Button("退格") { session.key(8) }
                    Button("Esc") { session.key(27) }
                    Button { session.wheel(120) } label: { Label("上滚", systemImage: "arrow.up") }
                    Button { session.wheel(-120) } label: { Label("下滚", systemImage: "arrow.down") }
                }.buttonStyle(.bordered).disabled(!session.canControl).padding(12)
            }
        }
        .background(Color(uiColor: .systemBackground))
        .onChange(of: session.canControl) { streaming in UIApplication.shared.isIdleTimerDisabled = streaming }
        .onDisappear { UIApplication.shared.isIdleTimerDisabled = false }
    }
}

private extension View {
    func card() -> some View {
        padding(20).frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 20))
    }
}

private func hideKeyboard() {
    UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
}
