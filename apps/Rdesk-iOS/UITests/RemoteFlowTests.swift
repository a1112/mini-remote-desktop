import XCTest

final class RemoteFlowTests: XCTestCase {
    private struct Fixture: Decodable {
        let fingerprint: String
        let token: String
        let upgrades: Int
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testInvalidAddressStaysOnConnectionScreen() {
        let app = XCUIApplication()
        app.launch()
        let host = app.textFields["hostField"]
        XCTAssertTrue(host.waitForExistence(timeout: 10))
        host.tap()
        let current = host.value as? String ?? ""
        if current != "192.168.1.100" { host.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count)) }
        host.typeText("8.8.8.8")
        if app.buttons["完成"].exists { app.buttons["完成"].tap() }
        app.buttons["connectButton"].tap()
        XCTAssertTrue(app.staticTexts["connectionError"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["disconnectButton"].exists)
    }

    @MainActor
    func testFrameTouchKeyboardAndDisconnect() async throws {
        let fixture = try await fixtureInfo()
        let app = connectToFixture(fingerprint: fixture.fingerprint, token: fixture.token)
        XCTAssertTrue(app.staticTexts["streamStats"].waitForExistence(timeout: 15))
        let canvas = app.images["remoteCanvas"]
        XCTAssertTrue(canvas.exists)
        canvas.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.buttons["enterButton"].tap()
        app.buttons["keyboardButton"].tap()
        let field = app.textFields["remoteTextField"]
        field.tap()
        field.typeText("Rdesk iOS")
        app.buttons["sendTextButton"].tap()
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "iOS remote protocol fixture"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["disconnectButton"].tap()
        XCTAssertTrue(app.buttons["connectButton"].waitForExistence(timeout: 5))

        let data = try await fixtureData("events")
        let events = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        XCTAssertTrue(events.contains { $0["type"] as? String == "pointer" && $0["action"] as? String == "down" })
        XCTAssertTrue(events.contains { $0["type"] as? String == "pointer" && $0["action"] as? String == "up" })
        let keys = events.filter { $0["type"] as? String == "key" }
        XCTAssertEqual(keys.map { $0["pressed"] as? Bool }, [true, false])
        XCTAssertTrue(events.contains { $0["value"] as? String == "Rdesk iOS" })
    }

    @MainActor
    func testWrongCertificateStopsBeforeWebSocketUpgrade() async throws {
        let fixture = try await fixtureInfo()
        let alternateDigit = fixture.fingerprint.hasPrefix("0") ? "1" : "0"
        let differentPin = alternateDigit + fixture.fingerprint.dropFirst()
        let app = connectToFixture(fingerprint: differentPin, token: fixture.token)
        assertFailed(app, contains: "证书指纹不匹配")
        let after = try await fixtureInfo()
        XCTAssertEqual(after.upgrades, fixture.upgrades)
    }

    @MainActor
    func testValidCertificateDoesNotAcceptWrongPairingToken() async throws {
        let fixture = try await fixtureInfo()
        let app = connectToFixture(fingerprint: fixture.fingerprint, token: String(repeating: "x", count: 32))
        assertFailed(app, contains: "连接未建立")
        let after = try await fixtureInfo()
        XCTAssertGreaterThan(after.upgrades, fixture.upgrades)
    }

    @MainActor
    private func connectToFixture(fingerprint: String, token: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--gateway-fixture"]
        app.launchEnvironment["MRD_IOS_FIXTURE_FINGERPRINT"] = fingerprint
        app.launchEnvironment["MRD_IOS_FIXTURE_TOKEN"] = token
        app.launch()
        let fingerprintField = app.textFields["fingerprintField"]
        XCTAssertTrue(fingerprintField.waitForExistence(timeout: 10))
        XCTAssertEqual(fingerprintField.value as? String, fingerprint)
        app.buttons["connectButton"].tap()
        return app
    }

    @MainActor
    private func assertFailed(_ app: XCUIApplication, contains message: String) {
        let status = app.staticTexts["sessionStatus"]
        let failed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label CONTAINS %@", message), object: status)
        XCTAssertEqual(XCTWaiter.wait(for: [failed], timeout: 8), .completed)
        XCTAssertFalse(app.staticTexts["streamStats"].exists)
    }

    private func fixtureInfo() async throws -> Fixture {
        try JSONDecoder().decode(Fixture.self, from: await fixtureData("health"))
    }

    private func fixtureData(_ path: String) async throws -> Data {
        let url = URL(string: "http://127.0.0.1:19535/\(path)")!
        let request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 5)
        let (data, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
        return data
    }
}
