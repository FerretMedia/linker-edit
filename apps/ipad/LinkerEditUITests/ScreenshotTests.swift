import UIKit
import XCTest

/// Drives the app on the iPad simulator and keeps screenshots, which CI uploads (see .github/workflows/ipad.yml).
final class ScreenshotTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        // Landscape, as an iPad sits in a keyboard case.
        XCUIDevice.shared.orientation = .landscapeLeft
    }

    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = arguments
        app.launch()
        return app
    }

    private func screenshot(_ name: String, of app: XCUIApplication) {
        let window = app.windows.firstMatch.frame
        let attachment = XCTAttachment(image: upright(XCUIScreen.main.screenshot().image, landscape: window.width > window.height))
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// The simulator's screenshots come in the screen's portrait orientation even when the app runs in landscape
    /// (the attachment's PNG drops any orientation flag); this turns them the way the app is shown.
    private func upright(_ screenshot: UIImage, landscape: Bool) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = screenshot.scale
        // Bake in an orientation flag, if there is one, so the pixels are what the size says.
        let image = screenshot.imageOrientation == .up ? screenshot : UIGraphicsImageRenderer(size: screenshot.size, format: format).image { _ in
            screenshot.draw(in: CGRect(origin: .zero, size: screenshot.size))
        }
        guard landscape, image.size.height > image.size.width else { return image }
        let size = CGSize(width: image.size.height, height: image.size.width)
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            if XCUIDevice.shared.orientation == .landscapeRight {
                context.cgContext.translateBy(x: size.width, y: 0)
                context.cgContext.rotate(by: .pi / 2)
            } else {
                context.cgContext.translateBy(x: 0, y: size.height)
                context.cgContext.rotate(by: -.pi / 2)
            }
            image.draw(at: .zero)
        }
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
        screenshot("1-connect", of: app)
    }

    func testChatWithToolCardsAndAnApproval() {
        let app = launch(["-demo", "-uiTestAppearance", "light"])
        XCTAssertTrue(app.staticTexts["Cut the silences longer than a second out of the interview."].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["allow"].exists)
        XCTAssertTrue(app.buttons["deny"].exists)
        XCTAssertTrue(app.buttons["resolve-status"].exists)
        screenshot("2-chat", of: app)

        app.buttons["tool-t1"].tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "00:12:31:04")).firstMatch.waitForExistence(timeout: 5))
        screenshot("3-tool-open", of: app)

        app.buttons["preview-toggle"].tap()
        XCTAssertTrue(app.buttons["Render preview"].waitForExistence(timeout: 5))
        screenshot("4-preview", of: app)
    }

    /// The same chat in dark mode (the app follows the system's, but takes -uiTestAppearance for screenshots).
    func testChatInDarkMode() {
        let app = launch(["-demo", "-uiTestAppearance", "dark"])
        XCTAssertTrue(app.staticTexts["Cut the silences longer than a second out of the interview."].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["allow"].exists)
        screenshot("2-chat-dark", of: app)
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
        screenshot("5-real-server", of: app)
        app.buttons["disconnect"].tap()
        XCTAssertTrue(app.textFields["host-field"].waitForExistence(timeout: 10))
    }
}
