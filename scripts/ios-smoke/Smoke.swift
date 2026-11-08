import XCTest
final class Smoke: XCTestCase {
    func record(_ name: String, _ app: XCUIApplication) {
        let tree = XCTAttachment(string: app.debugDescription)
        tree.name = name + "-tree"; tree.lifetime = .keepAlways; add(tree)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = name; screenshot.lifetime = .keepAlways; add(screenshot)
    }

    override func tearDown() {
        record("final-peazip", XCUIApplication(bundleIdentifier: "io.peazip.rust"))
        super.tearDown()
    }

    func testImportAndNavigate() {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "io.peazip.rust")
        app.launch()
        let open = app.buttons["Open archive…"]
        XCTAssertTrue(open.waitForExistence(timeout: 10), app.debugDescription)
        open.tap()
        XCTAssertTrue(app.buttons["Cancel"].waitForExistence(timeout: 15), app.debugDescription)
        record("picker-recents", app)
        let sample = app.cells.containing(.staticText, identifier: "__FIXTURE_NAME__").firstMatch
        if !sample.exists {
            let browse = app.buttons.matching(NSPredicate(format: "label IN %@", ["Browse", "浏览"])).firstMatch
            if browse.exists { browse.tap() }
            let downloads = app.cells.containing(NSPredicate(format: "label IN %@", ["Downloads", "下载项", "下载"])).firstMatch
            if downloads.exists { downloads.tap() }
        }
        record("picker-browse", app)
        XCTAssertTrue(sample.waitForExistence(timeout: 15), app.debugDescription)
        sample.tap()
        XCTAssertTrue(app.staticTexts["5 entries"].waitForExistence(timeout: 20), app.debugDescription)
        let docs = app.buttons["Open folder docs/"]
        XCTAssertTrue(docs.waitForExistence(timeout: 10), app.debugDescription)
        docs.tap()
        XCTAssertTrue(app.buttons["Open folder docs/nested/"].waitForExistence(timeout: 10), app.debugDescription)
        let chinese = app.buttons.matching(NSPredicate(format: "label == %@", "docs/你好.txt")).firstMatch
        XCTAssertTrue(chinese.waitForExistence(timeout: 10), app.debugDescription)
        let row = app.otherElements.matching(NSPredicate(format: "label == %@", "你好.txt Size in bytes: 11")).firstMatch
        XCTAssertTrue(row.exists, app.debugDescription)
        XCTAssertTrue(row.buttons.matching(NSPredicate(format: "label == %@", "docs/你好.txt")).firstMatch.exists)
        record("imported-chinese-row", app)
        app.buttons["Up"].tap()
        XCTAssertTrue(docs.waitForExistence(timeout: 10))
        app.buttons["Back"].tap()
        XCTAssertTrue(chinese.waitForExistence(timeout: 10))
        record("navigation-back", app)
    }

    func testOpenAndCancel() {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "io.peazip.rust")
        app.activate()
        let open = app.buttons["Open archive…"]
        XCTAssertTrue(open.waitForExistence(timeout: 15), app.debugDescription)
        record("cancel-initial", app)
        for attempt in 1...2 {
            let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true AND hittable == true"), object: open)
            XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 10), .completed)
            open.tap()
            let cancel = app.buttons["Cancel"]
            XCTAssertTrue(cancel.waitForExistence(timeout: 15), app.debugDescription)
            record("picker-open-\(attempt)", app)
            cancel.tap()
            let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: cancel)
            XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 10), .completed, app.debugDescription)
            let returned = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true AND hittable == true"), object: open)
            XCTAssertEqual(XCTWaiter.wait(for: [returned], timeout: 10), .completed, app.debugDescription)
            record("picker-cancelled-\(attempt)", app)
        }
    }
}
