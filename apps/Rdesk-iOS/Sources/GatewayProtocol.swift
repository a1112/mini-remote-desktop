import Foundation
import CoreGraphics

enum GatewayError: LocalizedError {
    case invalidAddress, invalidPort, invalidFrame, invalidMessage, invalidText
    case server(String)

    var errorDescription: String? {
        switch self {
        case .invalidAddress: return "请输入受信局域网的 IPv4 地址，如 192.168.1.100。"
        case .invalidPort: return "端口必须是 1 到 65535 的整数。"
        case .invalidFrame: return "电脑返回了无效或过大的画面。"
        case .invalidMessage: return "电脑返回了无法识别的网关消息。"
        case .invalidText: return "文字不能为空，且不能超过 512 字节。"
        case .server(let message):
            if message == "Windows capture is required" {
                return "此移动网关需要运行在 Windows 电脑上。"
            }
            return message
        }
    }
}

struct GatewayEndpoint: Equatable {
    let host: String
    let port: UInt16

    init(host: String, port: String) throws {
        let trimmed = host.trimmingCharacters(in: .whitespacesAndNewlines)
        let parts = trimmed.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { throw GatewayError.invalidAddress }
        let octets = try parts.map { part -> UInt8 in
            guard !part.isEmpty, part.count <= 3,
                  part.allSatisfy({ $0.isASCII && $0.isNumber }),
                  let value = UInt8(part) else { throw GatewayError.invalidAddress }
            return value
        }
        guard octets[0] == 10 || octets[0] == 127 ||
                (octets[0] == 172 && (16...31).contains(octets[1])) ||
                (octets[0] == 192 && octets[1] == 168) else {
            throw GatewayError.invalidAddress
        }
        let portText = port.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !portText.isEmpty, portText.allSatisfy({ $0.isASCII && $0.isNumber }),
              let number = UInt16(portText), number > 0 else { throw GatewayError.invalidPort }
        // Canonicalize leading zeros so URLs cannot interpret an address as octal.
        self.host = octets.map(String.init).joined(separator: ".")
        self.port = number
    }

    var url: URL { URL(string: "ws://\(host):\(port)/mobile/desktop/ws")! }
}

enum GatewayProtocol {
    static let maxJPEGBytes = 2 * 1024 * 1024

    static func validateJPEG(_ data: Data) throws {
        guard (4...maxJPEGBytes).contains(data.count),
              data.starts(with: [0xff, 0xd8]), data.suffix(2) == Data([0xff, 0xd9]) else {
            throw GatewayError.invalidFrame
        }
    }

    enum Event: Equatable {
        case ready, dimensions(Int, Int), error(String), pong
    }

    static func event(_ text: String) throws -> Event {
        guard text.utf8.count <= 4096,
              let data = text.data(using: .utf8),
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = object["type"] as? String else { throw GatewayError.invalidMessage }
        switch type {
        case "ready": return .ready
        case "error": return .error(object["message"] as? String ?? "网关拒绝连接。")
        case "pong": return .pong
        case "dimensions":
            guard let w = object["width"] as? Int, let h = object["height"] as? Int,
                  (1...32768).contains(w), (1...32768).contains(h) else {
                throw GatewayError.invalidMessage
            }
            return .dimensions(w, h)
        default: throw GatewayError.invalidMessage
        }
    }

    static func textControl(_ text: String) throws -> [String: Any] {
        guard !text.isEmpty, text.utf8.count <= 512 else { throw GatewayError.invalidText }
        return ["type": "text", "value": text]
    }

    static func control(_ object: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        // Gateway rejects messages over 1024 bytes, including JSON escaping.
        guard data.count <= 1024 else { throw GatewayError.invalidText }
        return String(decoding: data, as: UTF8.self)
    }
}

enum RemoteGeometry {
    static func imageRect(image: CGSize, bounds: CGRect) -> CGRect {
        guard image.width > 0, image.height > 0, bounds.width > 0, bounds.height > 0 else { return .zero }
        let scale = min(bounds.width / image.width, bounds.height / image.height)
        let size = CGSize(width: image.width * scale, height: image.height * scale)
        return CGRect(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2,
                      width: size.width, height: size.height)
    }

    static func normalized(_ point: CGPoint, in rect: CGRect, clamp: Bool) -> CGPoint? {
        guard rect.width > 0, rect.height > 0, point.x.isFinite, point.y.isFinite,
              clamp || rect.contains(point) else { return nil }
        return CGPoint(x: min(1, max(0, (point.x - rect.minX) / rect.width)),
                       y: min(1, max(0, (point.y - rect.minY) / rect.height)))
    }
}
