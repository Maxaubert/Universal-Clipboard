import XCTest

/// Opens the Share sheet in the host app and picks the S5 extension, so its start can be
/// measured in its own process (log lines "S5EXT").
final class ShareTests: XCTestCase {
    func testShareExtensionStarts() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-shareTest"]
        app.launch()
        app.buttons["Share"].tap()
        let name = "UC S5"
        var cell = app.cells[name]
        if !cell.waitForExistence(timeout: 5) {
            // Not in the first row: open "More" (Edit Actions list) and pick it there.
            let more = app.cells["More"]
            if more.waitForExistence(timeout: 3) { more.tap() }
            cell = app.cells[name]
        }
        XCTAssertTrue(cell.waitForExistence(timeout: 5), app.debugDescription)
        cell.tap()
        sleep(4)
    }
}
