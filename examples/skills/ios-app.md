# iOS app
Description: Create, build and run an iOS app in the Simulator from the terminal, and test its logic — no Xcode window needed.

No command creates an Xcode project: `xcodebuild -create-project` does not exist. Build the app without a project, as below. Replace NAME with the app's name and BUNDLE_ID with an id such as `dev.example.NAME`.

## 1. Write the app

SwiftUI source files in `App/`. One of them holds the entry point:

```swift
import SwiftUI

@main
struct NAMEApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}
```

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
xcrun simctl launch booted BUNDLE_ID                      # prints BUNDLE_ID: <pid>
sleep 3
xcrun simctl spawn booted launchctl list | grep BUNDLE_ID # still listed: it did not crash
xcrun simctl io booted screenshot screenshot.png
```

You cannot see images. Say the screenshot was taken and where, so the user can look; do not describe how the screen looks.

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

## 5. Report

Say exactly what happened: whether it compiled, whether it launched (the pid), whether it was still running a few seconds later, where the screenshot is, and how many tests ran and passed. Say what was not checked: how the screen looks.

## If the user wants an Xcode project

A real `.xcodeproj` needs XcodeGen (`brew install xcodegen`; ask before installing anything). Write a `project.yml`, run `xcodegen generate`, then `xcodebuild -scheme NAME -destination 'platform=iOS Simulator,name=DEVICE NAME' build test`.
