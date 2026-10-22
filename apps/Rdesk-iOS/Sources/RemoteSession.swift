import Foundation
import UIKit
import ImageIO

private actor FrameDecoder {
    func decode(_ data: Data) throws -> UIImage {
        try GatewayProtocol.validateJPEG(data)
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 8192, height <= 8192,
              width * height <= 20_000_000,
              let image = CGImageSourceCreateImageAtIndex(source, 0,
                [kCGImageSourceShouldCacheImmediately: true] as CFDictionary) else {
            throw GatewayError.invalidFrame
        }
        return UIImage(cgImage: image)
    }
}

// Reject redirects so a LAN endpoint cannot send this cleartext client to a public host.
private final class LANSessionDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

@MainActor
final class RemoteSession: ObservableObject {
    enum Phase: Equatable { case idle, connecting, waiting, streaming, failed }
    @Published private(set) var phase: Phase = .idle
    @Published private(set) var message = "准备连接"
    @Published private(set) var image: UIImage?
    @Published private(set) var frames = 0
    @Published private(set) var dimensions = ""
    @Published private(set) var fps = 0
    @Published var inputError: String?

    var active: Bool { [.connecting, .waiting, .streaming].contains(phase) }
    var canControl: Bool { phase == .streaming }
    private var socket: URLSessionWebSocketTask?
    private var transport: URLSession?
    private var receiver: Task<Void, Never>?
    private var writer: Task<Void, Never>?
    private var watchdog: Task<Void, Never>?
    private var generation = UUID()
    private var pending: [(text: String, move: Bool)] = []
    private var lastFrameAt = Date()
    private var fpsStart = Date()
    private var fpsFrames = 0
    private let decoder = FrameDecoder()

    func connect(_ endpoint: GatewayEndpoint) {
        close()
        phase = .connecting
        message = "正在连接电脑…"
        frames = 0
        fps = 0
        dimensions = ""
        inputError = nil
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 12
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        let session = URLSession(configuration: config, delegate: LANSessionDelegate(), delegateQueue: nil)
        transport = session
        let task = session.webSocketTask(with: endpoint.url)
        task.maximumMessageSize = GatewayProtocol.maxJPEGBytes
        socket = task
        let id = generation
        lastFrameAt = Date()
        fpsStart = Date()
        fpsFrames = 0
        task.resume()
        receiver = Task { [weak self] in
            do {
                while !Task.isCancelled {
                    let incoming = try await task.receive()
                    guard let self, self.generation == id else { return }
                    switch incoming {
                    case .string(let text):
                        switch try GatewayProtocol.event(text) {
                        case .ready:
                            if self.phase == .connecting {
                                self.phase = .waiting
                                self.message = "已连接，等待电脑画面…"
                            }
                        case .dimensions(let w, let h): self.dimensions = "\(w) × \(h)"
                        case .error(let error): throw GatewayError.server(error)
                        case .pong: break
                        }
                    case .data(let data):
                        let decoded = try await self.decoder.decode(data)
                        guard self.generation == id, !Task.isCancelled else { return }
                        self.image = decoded
                        self.frames += 1
                        self.fpsFrames += 1
                        self.lastFrameAt = Date()
                        let elapsed = self.lastFrameAt.timeIntervalSince(self.fpsStart)
                        if elapsed >= 1 {
                            self.fps = Int((Double(self.fpsFrames) / elapsed).rounded())
                            self.fpsFrames = 0
                            self.fpsStart = self.lastFrameAt
                        }
                        self.phase = .streaming
                        self.message = "桌面已连接"
                    @unknown default: throw GatewayError.invalidMessage
                    }
                }
            } catch {
                guard let self, self.generation == id, !Task.isCancelled else { return }
                self.fail(error.localizedDescription)
            }
        }
        watchdog = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(nanoseconds: 2_000_000_000) } catch { return }
                guard let self, self.generation == id else { return }
                if Date().timeIntervalSince(self.lastFrameAt) >= 12 {
                    self.fail("12 秒未收到电脑画面，请检查电脑网关、网络和屏幕采集权限。")
                    return
                }
            }
        }
    }

    func disconnect(message: String = "已断开连接") {
        close()
        phase = .idle
        self.message = message
    }

    private func fail(_ reason: String) {
        close()
        phase = .failed
        message = reason
    }

    private func close() {
        generation = UUID()
        receiver?.cancel()
        writer?.cancel()
        watchdog?.cancel()
        receiver = nil
        writer = nil
        watchdog = nil
        // Gateway releases every held button/key when the WebSocket closes.
        socket?.cancel(with: .normalClosure, reason: nil)
        transport?.invalidateAndCancel()
        socket = nil
        transport = nil
        pending.removeAll()
        image = nil
    }

    func pointer(_ point: CGPoint, action: String) {
        guard point.x.isFinite, point.y.isFinite,
              (0...1).contains(point.x), (0...1).contains(point.y),
              ["down", "move", "up"].contains(action) else { return }
        enqueue(["type": "pointer", "x": point.x, "y": point.y, "action": action], move: action == "move")
    }

    func wheel(_ delta: Int) { enqueue(["type": "wheel", "delta": min(1200, max(-1200, delta))]) }

    func key(_ code: UInt16) {
        guard code > 0 else { return }
        enqueue(["type": "key", "code": code, "pressed": true])
        enqueue(["type": "key", "code": code, "pressed": false])
    }

    @discardableResult
    func sendText(_ text: String) -> Bool {
        guard canControl else { return false }
        do {
            let control = try GatewayProtocol.textControl(text)
            _ = try GatewayProtocol.control(control)
            enqueue(control)
            inputError = nil
            return true
        } catch {
            inputError = error.localizedDescription
            return false
        }
    }

    private func enqueue(_ object: [String: Any], move: Bool = false) {
        guard canControl, let socket else { return }
        do {
            let text = try GatewayProtocol.control(object)
            if move, pending.last?.move == true { pending.removeLast() }
            guard pending.count < 64 else {
                fail("网络发送过慢，连接已断开以释放电脑上的按键。")
                return
            }
            pending.append((text, move))
            guard writer == nil else { return }
            let id = generation
            writer = Task { [weak self] in
                do {
                    while let self, self.generation == id, !Task.isCancelled, !self.pending.isEmpty {
                        let item = self.pending.removeFirst()
                        try await socket.send(.string(item.text))
                    }
                    if let self, self.generation == id { self.writer = nil }
                } catch {
                    guard let self, self.generation == id, !Task.isCancelled else { return }
                    self.fail("发送操作失败：\(error.localizedDescription)")
                }
            }
        } catch { inputError = error.localizedDescription }
    }
}
