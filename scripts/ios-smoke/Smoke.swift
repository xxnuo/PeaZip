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

    func waitReady(_ element: XCUIElement, _ app: XCUIApplication) {
        if element.exists && element.isEnabled && element.isHittable { return }
        XCTAssertTrue(element.waitForExistence(timeout: 15), app.debugDescription)
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true AND hittable == true"), object: element)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 15), .completed, app.debugDescription)
    }

    func waitPicker(_ app: XCUIApplication) {
        let picker = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "DOC.browsingRoot")).firstMatch
        XCTAssertTrue(picker.waitForExistence(timeout: 15), app.debugDescription)
    }

    func cancelPicker(_ app: XCUIApplication) {
        for _ in 0..<6 {
            let cancel = app.buttons.matching(NSPredicate(format: "label IN %@", ["Cancel", "取消"])).firstMatch
            if cancel.waitForExistence(timeout: 3) {
                waitReady(cancel, app)
                cancel.tap()
                return
            }
            let back = app.buttons.matching(identifier: "BackButton").firstMatch
            if back.exists {
                waitReady(back, app)
                back.tap()
            } else {
                app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.08))
                    .press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)))
                return
            }
        }
        XCTFail("No system picker cancellation action after returning through its parent folders: " + app.debugDescription)
    }

    func localRoot(_ app: XCUIApplication) {
        if app.cells.matching(identifier: "PeaZip, Container").firstMatch.exists { return }
        let browse = app.buttons.matching(NSPredicate(format: "label IN %@", ["Browse", "浏览"])).firstMatch
        if browse.exists && browse.isHittable { browse.tap() }
        let local = app.cells.matching(NSPredicate(format: "label IN %@", ["On My iPhone", "我的iPhone"])).firstMatch
        for _ in 0..<6 {
            if app.cells.matching(identifier: "PeaZip, Container").firstMatch.exists { return }
            if local.waitForExistence(timeout: 2) {
                waitReady(local, app)
                local.tap()
                return
            }
            let back = app.buttons.matching(identifier: "BackButton").firstMatch
            if back.exists && back.isHittable { back.tap() } else { return }
        }
        XCTFail("Could not navigate to the local Files provider: " + app.debugDescription)
    }

    func downloads(_ app: XCUIApplication) {
        localRoot(app)
        let folder = app.cells.containing(NSPredicate(format: "label IN %@", ["Downloads", "下载项", "下载"])).firstMatch
        if folder.waitForExistence(timeout: 5) && folder.isHittable { folder.tap() }
    }

    func createdRows(_ app: XCUIApplication) {
        for name in ["输入你好.txt", "data.bin", "empty.txt"] {
            XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label == %@", name)).firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        }
    }

    func chooseCreateInputs(_ app: XCUIApplication, _ format: String) {
        waitReady(app.buttons["Create archive…"], app)
        app.buttons["Create archive…"].tap()
        waitReady(app.buttons[format], app)
        app.buttons[format].tap()
        app.buttons["Continue"].tap()
        waitPicker(app)
        downloads(app)
        let inputFolder = app.cells.containing(.staticText, identifier: "__CREATE_INPUT_FOLDER__").firstMatch
        waitReady(inputFolder, app)
        inputFolder.tap()
        for name in ["输入你好.txt", "data.bin", "empty.txt"] {
            let basename = (name as NSString).deletingPathExtension
            let full = app.cells.containing(.staticText, identifier: name)
            let stem = app.cells.containing(.staticText, identifier: basename)
            let candidates = full.count > 0 ? full : stem
            XCTAssertEqual(candidates.count, 1, "Each real input must match exactly one picker cell: " + name)
            let target = candidates.firstMatch
            waitReady(target, app)
            target.tap()
        }
        record("selected-inputs-" + format, app)
        let open = app.buttons.matching(NSPredicate(format: "label IN %@", ["Open", "打开"])).firstMatch
        waitReady(open, app)
        open.tap()
        XCTAssertTrue(app.staticTexts["Created " + format + ": 3 entries"].waitForExistence(timeout: 20), app.debugDescription)
        createdRows(app)
        record("created-" + format, app)
    }

    func saveCreated(_ app: XCUIApplication, _ format: String, cancel: Bool = false) {
        let saveArchive = app.buttons["Save " + format + "…"]
        waitReady(saveArchive, app)
        saveArchive.tap()
        waitPicker(app)
        record("export-picker-" + format + (cancel ? "-cancel" : ""), app)
        if cancel {
            cancelPicker(app)
            XCTAssertTrue(app.staticTexts["Save cancelled."].waitForExistence(timeout: 10), app.debugDescription)
        } else {
            localRoot(app)
            record("export-local-provider-root-" + format, app)
            let save = app.buttons.matching(NSPredicate(format: "label IN %@", ["Save", "保存", "Export", "导出", "Move", "移动", "Copy", "拷贝"])).firstMatch
            waitReady(save, app)
            save.tap()
            XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Saved archive:")).firstMatch.waitForExistence(timeout: 15), app.debugDescription)
        }
        waitReady(saveArchive, app)
        createdRows(app)
    }

    func testCreateAndSaveFormats() {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "io.peazip.rust")
        app.launch()
        for format in ["ZIP", "TAR", "TAR.GZ"] {
            chooseCreateInputs(app, format)
            let selected = app.buttons.matching(NSPredicate(format: "label == %@", "输入你好.txt")).firstMatch
            selected.tap()
            XCTAssertTrue(app.staticTexts["1 selected"].waitForExistence(timeout: 10))
            saveCreated(app, format)
            record("saved-" + format, app)
            app.buttons["Create archive…"].tap()
            app.buttons["Cancel"].tap()
            createdRows(app)
            XCTAssertTrue(app.staticTexts["1 selected"].exists)
            app.buttons["Create archive…"].tap()
            app.buttons["Continue"].tap()
            waitPicker(app)
            cancelPicker(app)
            waitReady(app.buttons["Create archive…"], app)
            createdRows(app)
            XCTAssertTrue(app.staticTexts["1 selected"].exists)
            saveCreated(app, format, cancel: true)
            XCTAssertTrue(app.staticTexts["1 selected"].exists)
        }
    }

    func testExportCancellationGesture() {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "io.peazip.rust")
        app.activate()
        let picker = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "DOC.browsingRoot")).firstMatch
        let save = app.buttons.matching(NSPredicate(format: "label IN %@", ["Save ZIP…", "Save TAR…", "Save TAR.GZ…", "Save 7Z…"])).firstMatch
        if !picker.exists {
            waitReady(save, app)
            save.tap()
            waitPicker(app)
        }
        record("before-export-dismiss-gesture", app)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.08))
            .press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)))
        XCTAssertTrue(app.staticTexts["Save cancelled."].waitForExistence(timeout: 15), app.debugDescription)
        waitReady(save, app)
        createdRows(app)
        XCTAssertTrue(app.staticTexts["1 selected"].exists, app.debugDescription)
        record("after-export-dismiss-gesture", app)
    }

    func testSaveExistingZip() {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "io.peazip.rust")
        app.activate()
        let picker = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "DOC.browsingRoot")).firstMatch
        if picker.exists { cancelPicker(app) }
        createdRows(app)
        saveCreated(app, "ZIP")
        XCTAssertTrue(app.staticTexts["1 selected"].exists, app.debugDescription)
        record("existing-zip-exported", app)
    }

    func archiveSnapshot(_ app: XCUIApplication) -> [String: String] {
        let table = app.otherElements.matching(NSPredicate(format: "label == %@", "Archive contents")).firstMatch
        XCTAssertTrue(table.exists, app.debugDescription)
        var snapshot = ["archive": app.textFields["Archive path"].value as? String ?? ""]
        for (index, row) in table.buttons.allElementsBoundByIndex.enumerated() {
            snapshot["row-\(index)-" + row.label] = row.value as? String ?? ""
        }
        snapshot["selection"] = app.staticTexts.matching(NSPredicate(format: "label ENDSWITH %@", " selected")).firstMatch.label
        snapshot["filter"] = app.textFields["Filter names"].value as? String ?? ""
        return snapshot
    }

    func extractChoice(_ label: String, _ app: XCUIApplication) {
        let form = app.otherElements.matching(NSPredicate(format: "label == %@", "Extract archive")).firstMatch
        let choices = form.buttons.matching(NSPredicate(format: "label == %@", label))
            .allElementsBoundByIndex.filter { $0.isEnabled && $0.isHittable }
        XCTAssertEqual(choices.count, 1, "Expected one visible enabled extraction choice: " + label + "\n" + app.debugDescription)
        guard let choice = choices.first else { return }
        waitReady(choice, app)
        choice.tap()
    }

    func extractForm(_ app: XCUIApplication, scope: String, paths: String) {
        waitReady(app.buttons["Extract…"], app)
        app.buttons["Extract…"].tap()
        waitReady(app.buttons["Continue"], app)
        extractChoice(scope, app)
        extractChoice(paths, app)
    }

    func extractCase(_ fixture: String, scope: String, paths: String, filtered: Bool = false, cancellations: Bool = false) {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: "io.peazip.rust")
        app.launch()
        waitReady(app.buttons["Open archive…"], app)
        app.buttons["Open archive…"].tap()
        waitPicker(app)
        downloads(app)
        let sample = app.cells.containing(.staticText, identifier: fixture).firstMatch
        waitReady(sample, app)
        sample.tap()
        let docs = app.buttons.matching(NSPredicate(format: "label == %@", "docs/")).firstMatch
        waitReady(docs, app)
        docs.tap()
        XCTAssertTrue(app.staticTexts["1 selected"].waitForExistence(timeout: 10), app.debugDescription)
        if filtered {
            app.buttons["Open folder docs/"].tap()
            let filter = app.textFields["Filter names"]
            waitReady(filter, app)
            filter.tap()
            filter.typeText("你好\n")
            XCTAssertTrue(app.staticTexts["1 of 3 rows shown"].waitForExistence(timeout: 10), app.debugDescription)
        }
        let before = archiveSnapshot(app)
        if cancellations {
            extractForm(app, scope: scope, paths: "Files only (no folders)")
            app.buttons["Cancel"].tap()
            waitReady(app.buttons["Extract…"], app)
            XCTAssertEqual(archiveSnapshot(app), before, app.debugDescription)
            record("extract-form-cancelled", app)
            for attempt in 1...2 {
                extractForm(app, scope: scope, paths: paths)
                app.buttons["Continue"].tap()
                waitPicker(app)
                record("extract-system-cancel-\(attempt)", app)
                cancelPicker(app)
                XCTAssertTrue(app.staticTexts["Extraction cancelled."].waitForExistence(timeout: 15), app.debugDescription)
                waitReady(app.buttons["Extract…"], app)
                XCTAssertEqual(archiveSnapshot(app), before, app.debugDescription)
            }
        }
        extractForm(app, scope: scope, paths: paths)
        if filtered {
            XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", "Current view: /docs/", "你好")).firstMatch.exists, app.debugDescription)
            XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "1 in displayed scope")).firstMatch.exists, app.debugDescription)
        }
        record("extract-frozen-form-" + fixture + "-" + scope + "-" + paths, app)
        app.buttons["Continue"].tap()
        waitPicker(app)
        localRoot(app)
        record("extract-provider-root", app)
        let save = app.buttons.matching(NSPredicate(format: "label IN %@", ["Save", "保存", "Export", "导出", "Copy", "拷贝"])).firstMatch
        waitReady(save, app)
        save.tap()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Extracted ")).firstMatch.waitForExistence(timeout: 20), app.debugDescription)
        waitReady(app.buttons["Extract…"], app)
        XCTAssertEqual(archiveSnapshot(app), before, app.debugDescription)
        record("extract-exported-" + fixture + "-" + scope + "-" + paths, app)
    }

    func testExtractSelectedDirectory() {
        extractCase("__EXTRACT_TAR_NAME__", scope: "Selected entries", paths: "Full paths", cancellations: true)
    }

    func testExtractFilesOnly() {
        extractCase("__EXTRACT_7Z_NAME__", scope: "Selected entries", paths: "Files only (no folders)")
    }

    func testExtractDisplayedFrozen() {
        extractCase("__EXTRACT_TAR_NAME__", scope: "Displayed entries", paths: "Full paths", filtered: true)
    }

    func testExtractAll() {
        extractCase("__EXTRACT_7Z_NAME__", scope: "All entries", paths: "Full paths")
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
        if !sample.exists { downloads(app) }
        record("picker-browse", app)
        waitReady(sample, app)
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
        let picker = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "DOC.browsingRoot")).firstMatch
        if picker.exists { cancelPicker(app) }
        waitReady(open, app)
        let table = app.otherElements.matching(NSPredicate(format: "label == %@", "Archive contents")).firstMatch
        XCTAssertTrue(table.exists, app.debugDescription)
        let rowsBefore = Dictionary(uniqueKeysWithValues: table.buttons.allElementsBoundByIndex.map { ($0.label, $0.value as? String ?? "") })
        let pathBefore = app.textFields["Archive path"].value as? String
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
            XCTAssertEqual(app.textFields["Archive path"].value as? String, pathBefore)
            XCTAssertEqual(Dictionary(uniqueKeysWithValues: table.buttons.allElementsBoundByIndex.map { ($0.label, $0.value as? String ?? "") }), rowsBefore)
            record("picker-cancelled-\(attempt)", app)
        }
    }
}
