import Foundation
import Network
import Darwin

struct DiscoveredGateway: Identifiable, Sendable {
    let endpoint: GatewayEndpoint
    let name: String
    let fingerprint: GatewayFingerprint
    var id: String { "\(endpoint.host):\(endpoint.port)" }

    static func decode(_ data: Data, from host: String) -> DiscoveredGateway? {
        struct Reply: Decodable {
            let type: String
            let port: UInt16
            let name: String?
            let tls_sha256: String
        }
        guard data.count <= 512,
              let reply = try? JSONDecoder().decode(Reply.self, from: data),
              reply.type == "rdesk_gateway",
              let endpoint = try? GatewayEndpoint(host: host, port: String(reply.port)),
              let fingerprint = try? GatewayFingerprint(reply.tls_sha256) else { return nil }
        let name = String((reply.name ?? "Rdesk 电脑").prefix(48))
        return DiscoveredGateway(endpoint: endpoint, name: name, fingerprint: fingerprint)
    }
}

@MainActor
final class LANDiscovery: ObservableObject {
    @Published private(set) var gateways: [DiscoveredGateway] = []
    @Published private(set) var scanning = false
    @Published private(set) var message = "在同一 Wi-Fi 下搜索，或手动输入电脑地址。"
    private var scan: Task<Void, Never>?
    private var generation = UUID()

    func stop() {
        generation = UUID()
        scan?.cancel()
        scan = nil
        scanning = false
    }

    func start(knownHost: String) {
        stop()
        gateways = []
        scanning = true
        message = "正在搜索局域网电脑…"
        let id = generation
        var targets: [String] = []
        if let known = try? GatewayEndpoint(host: knownHost, port: "9534") { targets.append(known.host) }
        // Unicast needs no restricted multicast entitlement. Bound the scan to
        // the Wi-Fi address's /24, and always offer manual entry for routed LANs.
        if let address = Self.wifiAddress() {
            let prefix = address.split(separator: ".").dropLast().joined(separator: ".")
            targets += (1...254).map { "\(prefix).\($0)" }.filter { $0 != address }
        }
        targets = Array(Set(targets)).sorted()
        scan = Task { [weak self] in
            for start in stride(from: 0, to: targets.count, by: 24) {
                guard !Task.isCancelled else { return }
                let batch = Array(targets[start..<min(start + 24, targets.count)])
                let found = await withTaskGroup(of: DiscoveredGateway?.self) { group in
                    for host in batch { group.addTask { await DiscoveryProbe(host: host).run() } }
                    var found: [DiscoveredGateway] = []
                    for await result in group { if let result { found.append(result) } }
                    return found
                }
                guard let self, self.generation == id, !Task.isCancelled else { return }
                self.gateways += found
                self.gateways.sort { $0.id < $1.id }
            }
            guard let self, self.generation == id, !Task.isCancelled else { return }
            self.scanning = false
            self.message = self.gateways.isEmpty
                ? "未发现电脑。请允许局域网访问、检查网关与防火墙，然后重新搜索；也可以手动连接。"
                : "发现 \(self.gateways.count) 台电脑，选择后核对证书指纹。"
        }
    }

    private static func wifiAddress() -> String? {
        var addresses: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&addresses) == 0 else { return nil }
        defer { freeifaddrs(addresses) }
        var current = addresses
        while let item = current {
            defer { current = item.pointee.ifa_next }
            guard String(cString: item.pointee.ifa_name) == "en0",
                  let address = item.pointee.ifa_addr,
                  address.pointee.sa_family == UInt8(AF_INET) else { continue }
            var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(address, socklen_t(address.pointee.sa_len), &buffer,
                           socklen_t(buffer.count), nil, 0, NI_NUMERICHOST) == 0 {
                let host = String(cString: buffer)
                if (try? GatewayEndpoint(host: host, port: "9534")) != nil { return host }
            }
        }
        return nil
    }
}

// Each probe owns a serial queue and completes exactly once, including cancel.
private final class DiscoveryProbe: @unchecked Sendable {
    private let host: String
    private let queue = DispatchQueue(label: "rdesk.discovery.probe")
    private var connection: NWConnection?
    private var continuation: CheckedContinuation<DiscoveredGateway?, Never>?
    private var cancelled = false

    init(host: String) { self.host = host }

    func run() async -> DiscoveredGateway? {
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                queue.async {
                    guard !self.cancelled else { continuation.resume(returning: nil); return }
                    self.continuation = continuation
                    let connection = NWConnection(host: .init(self.host), port: 9535, using: .udp)
                    self.connection = connection
                    connection.stateUpdateHandler = { state in
                        switch state {
                        case .ready:
                            connection.send(content: Data("MRD_DISCOVER_V1".utf8), completion: .contentProcessed { error in
                                if error != nil { self.finish(nil); return }
                                connection.receiveMessage { data, _, _, _ in
                                    self.finish(data.flatMap { DiscoveredGateway.decode($0, from: self.host) })
                                }
                            })
                        case .failed, .cancelled: self.finish(nil)
                        default: break
                        }
                    }
                    connection.start(queue: self.queue)
                    self.queue.asyncAfter(deadline: .now() + 0.8) { self.finish(nil) }
                }
            }
        } onCancel: {
            self.queue.async { self.cancelled = true; self.finish(nil) }
        }
    }

    private func finish(_ result: DiscoveredGateway?) {
        connection?.stateUpdateHandler = nil
        connection?.cancel()
        connection = nil
        let callback = continuation
        continuation = nil
        callback?.resume(returning: result)
    }
}
