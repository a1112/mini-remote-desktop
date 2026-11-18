import XCTest
@testable import RdeskMobile

final class GatewayProtocolTests: XCTestCase {
    func testPrivateAddressesAndCanonicalization() throws {
        for host in ["10.0.0.1", "172.16.0.1", "172.31.255.254", "192.168.1.3", "127.0.0.1"] {
            let endpoint = try GatewayEndpoint(host: host, port: "9534")
            XCTAssertEqual(endpoint.url.absoluteString, "wss://\(host):9534/mobile/desktop/ws")
        }
        XCTAssertEqual(try GatewayEndpoint(host: " 192.168.001.010 ", port: "9534").host, "192.168.1.10")
    }

    func testPinMatchesOnlyExactCertificate() throws {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        let pin = try GatewayFingerprint(" BA:78:" + digest.dropFirst(4).uppercased() + "\n")
        XCTAssertEqual(pin.hex, digest)
        XCTAssertTrue(pin.matches(certificate: Data("abc".utf8)))
        XCTAssertFalse(pin.matches(certificate: Data("new certificate".utf8)))
        XCTAssertThrowsError(try GatewayFingerprint(String(repeating: "g", count: 64)))
        XCTAssertThrowsError(try GatewayFingerprint(String(digest.dropLast())))
    }

    func testPairingTokenUsesHeaderOrLosslessQuery() throws {
        let endpoint = try GatewayEndpoint(host: "192.168.1.1", port: "9534")
        XCTAssertThrowsError(try endpoint.request(pairingToken: String(repeating: "a", count: 31)))
        let token = String(repeating: "a", count: 32) + "+/?#%="
        let request = try endpoint.request(pairingToken: token)
        XCTAssertEqual(request.value(forHTTPHeaderField: "X-MRD-Pairing-Token"), token)
        XCTAssertEqual(request.url, endpoint.url)

        let queryToken = String(repeating: "配", count: 11) + "+ /?#&=\n"
        let queryRequest = try endpoint.request(pairingToken: queryToken)
        XCTAssertNil(queryRequest.value(forHTTPHeaderField: "X-MRD-Pairing-Token"))
        let url = try XCTUnwrap(queryRequest.url)
        let query = try XCTUnwrap(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems)
        XCTAssertEqual(query, [URLQueryItem(name: "token", value: queryToken)])
    }

    func testDiscoveryRequiresValidCertificateFingerprint() throws {
        let pin = String(repeating: "af", count: 32)
        let reply = Data("{\"type\":\"rdesk_gateway\",\"port\":9534,\"name\":\"Office\",\"tls_sha256\":\"\(pin)\"}".utf8)
        let gateway = try XCTUnwrap(DiscoveredGateway.decode(reply, from: "192.168.1.2"))
        XCTAssertEqual(gateway.id, "192.168.1.2:9534")
        XCTAssertEqual(gateway.name, "Office")
        XCTAssertEqual(gateway.fingerprint.hex, pin)
        XCTAssertNil(DiscoveredGateway.decode(reply, from: "8.8.8.8"))
        let missingPin = Data("{\"type\":\"rdesk_gateway\",\"port\":9534}".utf8)
        XCTAssertNil(DiscoveredGateway.decode(missingPin, from: "192.168.1.2"))
        let invalidPin = Data("{\"type\":\"rdesk_gateway\",\"port\":9534,\"tls_sha256\":\"abc\"}".utf8)
        XCTAssertNil(DiscoveredGateway.decode(invalidPin, from: "192.168.1.2"))
    }

    func testRejectsPublicMalformedAndInjectedEndpoints() {
        for host in ["8.8.8.8", "172.15.1.1", "172.32.1.1", "192.169.1.1", "169.254.1.1",
                     "localhost", "::1", "192.168.1.256", "192.168.1", "192.168.1.1@evil.com",
                     "192.168.1.1/path", "192.168..1", "0xc0.168.1.1"] {
            XCTAssertThrowsError(try GatewayEndpoint(host: host, port: "9534"), host)
        }
        for port in ["", "0", "65536", "-1", "9534/path", "+9534"] {
            XCTAssertThrowsError(try GatewayEndpoint(host: "192.168.1.1", port: port))
        }
    }

    func testEventsMatchRustGateway() throws {
        XCTAssertEqual(try GatewayProtocol.event("{\"type\":\"ready\"}"), .ready)
        XCTAssertEqual(try GatewayProtocol.event("{\"type\":\"dimensions\",\"width\":1920,\"height\":1080}"), .dimensions(1920, 1080))
        XCTAssertEqual(try GatewayProtocol.event("{\"type\":\"error\",\"message\":\"Windows capture is required\"}"), .error("Windows capture is required"))
        XCTAssertThrowsError(try GatewayProtocol.event("{\"type\":\"dimensions\",\"width\":0,\"height\":1080}"))
        XCTAssertThrowsError(try GatewayProtocol.event("not json"))
    }

    func testBoundedJPEGAndText() throws {
        XCTAssertNoThrow(try GatewayProtocol.validateJPEG(Data([0xff, 0xd8, 0xff, 0xd9])))
        XCTAssertThrowsError(try GatewayProtocol.validateJPEG(Data([0xff, 0xd8])))
        XCTAssertThrowsError(try GatewayProtocol.validateJPEG(Data(repeating: 0, count: GatewayProtocol.maxJPEGBytes + 1)))
        let text = "你好，Windows 👋"
        let message = try GatewayProtocol.control(GatewayProtocol.textControl(text))
        let object = try JSONSerialization.jsonObject(with: Data(message.utf8)) as! [String: String]
        XCTAssertEqual(object, ["type": "text", "value": text])
        XCTAssertThrowsError(try GatewayProtocol.textControl(String(repeating: "你", count: 171)))
        XCTAssertThrowsError(try GatewayProtocol.textControl(""))
        XCTAssertThrowsError(try GatewayProtocol.control(GatewayProtocol.textControl(String(repeating: "\u{0}", count: 512))))
    }

    func testLetterboxingAndDragClamping() throws {
        let rect = RemoteGeometry.imageRect(image: CGSize(width: 1920, height: 1080),
                                            bounds: CGRect(x: 0, y: 0, width: 400, height: 400))
        XCTAssertEqual(rect, CGRect(x: 0, y: 87.5, width: 400, height: 225))
        XCTAssertNil(RemoteGeometry.normalized(CGPoint(x: 200, y: 30), in: rect, clamp: false))
        XCTAssertEqual(RemoteGeometry.normalized(CGPoint(x: 200, y: 200), in: rect, clamp: false), CGPoint(x: 0.5, y: 0.5))
        XCTAssertEqual(RemoteGeometry.normalized(CGPoint(x: -30, y: 800), in: rect, clamp: true), CGPoint(x: 0, y: 1))
        XCTAssertNil(RemoteGeometry.normalized(.zero, in: .zero, clamp: true))
    }
}
