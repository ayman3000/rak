# iOS app
Description: Create, build and run an iOS app in the Simulator from the terminal, and test its logic — no Xcode window needed.

No command creates an Xcode project: `xcodebuild -create-project` does not exist. Build the app without a project, as below. Replace NAME with the app's name and BUNDLE_ID with an id such as `dev.example.NAME`.

## 1. Write the app

SwiftUI source files in `App/`. One of them holds the entry point:

```swift
import SwiftUI

@main
struct NAMEApp: App {
    // Every print() reaches the log file of step 3 at once, not when a buffer fills.
    init() { setvbuf(stdout, nil, _IOLBF, 0) }
    var body: some Scene { WindowGroup { ContentView() } }
}
```

Never change what the app does just to test it. To open a particular screen for a test, have the app read a launch argument and pass it in step 3; without the argument the app starts as it always did:

```swift
// `-screen settings` at launch opens Settings; no argument: the usual first screen.
let screen = UserDefaults.standard.string(forKey: "screen")
```

Have the app `print` what a test needs to know: which screen opened, the values it fetched, the error when something fails. Print the values themselves, never your own verdict: a line such as "verified" or "ok" that is printed whatever happened checks nothing. Don't hide errors with `try?` in code a test relies on; catch them and print them.

Code that exists only for a test (fetching several cities at launch, say) runs only behind its own launch argument, such as `-test-fetch`. A normal launch must not run it.

Put logic that can be tested (models, conversions, formatting, parsing) in `Sources/NAMECore/` instead, as plain Swift with `public` types and functions, so step 4 can test it.

## 2. Build it for the Simulator

```bash
SDKV=$(xcrun --sdk iphonesimulator --show-sdk-version)
ARCH=$(uname -m)        # arm64 on Apple silicon, x86_64 on Intel
mkdir -p Build/NAME.app
xcrun --sdk iphonesimulator swiftc -parse-as-library -target $ARCH-apple-ios$SDKV-simulator \
  App/*.swift Sources/NAMECore/*.swift -o Build/NAME.app/NAME
```

The app's files are compiled together with the core's, so the app does not `import NAMECore`.

Write `Build/NAME.app/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>NAME</string>
  <key>CFBundleName</key><string>NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>UILaunchScreen</key><dict/>
</dict></plist>
```

## 3. Run it in the Simulator

```bash
xcrun simctl list devices available | grep -i iphone     # pick a device name
xcrun simctl boot "DEVICE NAME"     # "current state: Booted" means it already runs: fine
open -a Simulator
xcrun simctl install booted Build/NAME.app
xcrun simctl launch --terminate-running-process --stdout="$PWD/Build/run.log" booted BUNDLE_ID   # prints BUNDLE_ID: <pid>
sleep 3
xcrun simctl spawn booted launchctl list | grep BUNDLE_ID # still listed: it did not crash
cat Build/run.log                                          # what the app printed
xcrun simctl io booted screenshot Build/screen.png
```

To open another screen, add its launch argument at the end of the launch line: `... booted BUNDLE_ID -screen settings`. Keep the log under the project folder: the Simulator may not be able to write elsewhere.

Then look at the screenshot with `view_image` if you have it, and say what it shows. If you don't have `view_image`, you cannot see the screen: say the screenshot was taken and where, and do not describe how the screen looks.

Do all of step 3 again after every change to the app, not only the build.

## 4. Test the logic

`Package.swift` at the project's top:

```swift
// swift-tools-version:5.9
import PackageDescription
let package = Package(name: "NAMECore", platforms: [.macOS(.v13), .iOS(.v17)],
    targets: [.target(name: "NAMECore"),
              .testTarget(name: "NAMECoreTests", dependencies: ["NAMECore"])])
```

Tests in `Tests/NAMECoreTests/` with XCTest (`import XCTest` and `@testable import NAMECore`), then:

```bash
swift test
```

These run on the Mac, not in the Simulator: they test the logic, not the screens.

When the app calls a web API, check it before trusting it. Call the API with `curl` for two or three of the inputs (for weather: two or three cities) and look at the answers. Save one answer as a file in `Tests/NAMECoreTests/` and test the decoding against it in step 4. Then launch the app and read `Build/run.log` to see what it actually fetched.

## 5. Report

Say exactly what happened: whether it compiled, whether it launched (the pid), whether it was still running a few seconds later, what the log showed, what the screenshot shows (if you looked at it with `view_image`), and how many tests ran and passed. Say what was not checked.

## If the user wants an Xcode project

A real `.xcodeproj` needs XcodeGen (`brew install xcodegen`; ask before installing anything). Write a `project.yml`, run `xcodegen generate`, then `xcodebuild -scheme NAME -destination 'platform=iOS Simulator,name=DEVICE NAME' build test`.
