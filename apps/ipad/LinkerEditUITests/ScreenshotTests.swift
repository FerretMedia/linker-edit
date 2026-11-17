import XCTest

/// Drives the app on the iPad simulator and keeps screenshots, which CI uploads (see .github/workflows/ipad.yml).
final class ScreenshotTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .landscapeLeft
    }

    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = arguments
        app.launch()
        return app
    }

    private func screenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testConnectScreen() {
        let app = launch(["-reset"])
        let field = app.textFields["host-field"]
        XCTAssertTrue(field.waitForExistence(timeout: 15))
        XCTAssertFalse(app.buttons["connect-button"].isEnabled)
        field.tap()
        field.typeText("not a host at all")
        app.buttons["connect-button"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["connect-problem"].waitForExistence(timeout: 5))
        screenshot("1-connect")
    }

    func testChatWithToolCardsAndAnApproval() {
        let app = launch(["-demo"])
        XCTAssertTrue(app.staticTexts["Cut the silences longer than a second out of the interview."].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["allow"].exists)
        XCTAssertTrue(app.buttons["deny"].exists)
        XCTAssertTrue(app.buttons["resolve-status"].exists)
        screenshot("2-chat")

        app.buttons["tool-t1"].tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "00:12:31:04")).firstMatch.waitForExistence(timeout: 5))
        screenshot("3-tool-open")

        app.buttons["preview-toggle"].tap()
        XCTAssertTrue(app.buttons["Render preview"].waitForExistence(timeout: 5))
        screenshot("4-preview")
    }

    /// Against a real Linker-Edit server that CI starts on this Mac (LINKER_EDIT_URL).
    func testConnectsToARealServer() throws {
        guard let url = ProcessInfo.processInfo.environment["LINKER_EDIT_URL"] else {
            throw XCTSkip("Set LINKER_EDIT_URL (TEST_RUNNER_LINKER_EDIT_URL for xcodebuild) to a running Linker-Edit server.")
        }
        let app = launch(["-reset", "-host", url])
        // The pill only shows once the host's snapshot arrived and decoded.
        XCTAssertTrue(app.buttons["resolve-status"].waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts["Connected"].waitForExistence(timeout: 10))
        screenshot("5-real-server")
        app.buttons["disconnect"].tap()
        XCTAssertTrue(app.textFields["host-field"].waitForExistence(timeout: 10))
    }
}
