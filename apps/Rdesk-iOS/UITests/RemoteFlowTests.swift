import XCTest

final class RemoteFlowTests: XCTestCase {
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
        app.buttons["connectButton"].tap()
        XCTAssertTrue(app.staticTexts["connectionError"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["disconnectButton"].exists)
    }

    func testFrameTouchKeyboardAndDisconnect() throws {
        // scripts/test.sh supplies a loopback-only protocol fixture, never a real desktop.
        let app = XCUIApplication()
        app.launchArguments = ["--gateway-fixture"]
        app.launch()
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

        let completed = expectation(description: "Inspect actual WebSocket controls")
        URLSession.shared.dataTask(with: URL(string: "http://127.0.0.1:19534/events")!) { data, _, error in
            defer { completed.fulfill() }
            XCTAssertNil(error)
            guard let data, let events = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
                XCTFail("Fixture returned no events"); return
            }
            XCTAssertTrue(events.contains { $0["type"] as? String == "pointer" && $0["action"] as? String == "down" })
            XCTAssertTrue(events.contains { $0["type"] as? String == "pointer" && $0["action"] as? String == "up" })
            let keys = events.filter { $0["type"] as? String == "key" }
            XCTAssertEqual(keys.map { $0["pressed"] as? Bool }, [true, false])
            XCTAssertTrue(events.contains { $0["value"] as? String == "Rdesk iOS" })
        }.resume()
        wait(for: [completed], timeout: 5)
    }
}
