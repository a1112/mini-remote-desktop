import CoreGraphics
import Darwin
import Foundation

// Read only required window metadata for the exact process started by the
// smoke driver. No screenshots, accessibility APIs, or injected input.
guard CommandLine.arguments.count == 2,
      let pid = Int32(CommandLine.arguments[1]), pid > 0 else {
    FileHandle.standardError.write(Data("Expected one positive UI process ID\n".utf8))
    exit(2)
}

guard let windows = CGWindowListCopyWindowInfo(
    [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
) as? [[String: Any]] else {
    FileHandle.standardError.write(Data("A real GUI window-server session is required\n".utf8))
    exit(2)
}

let visibleWindows = windows.filter { window in
    guard let owner = window[kCGWindowOwnerPID as String] as? NSNumber,
          owner.int32Value == pid,
          let layer = window[kCGWindowLayer as String] as? NSNumber,
          layer.intValue == 0,
          let alpha = window[kCGWindowAlpha as String] as? NSNumber,
          alpha.doubleValue > 0,
          let bounds = window[kCGWindowBounds as String] as? [String: Any],
          let rectangle = CGRect(dictionaryRepresentation: bounds as CFDictionary) else {
        return false
    }
    return rectangle.width.isFinite && rectangle.height.isFinite
        && rectangle.width > 0 && rectangle.height > 0
}

let report: [String: Any] = ["pid": Int(pid), "visibleLayerZeroWindows": visibleWindows.count]
FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]))
FileHandle.standardOutput.write(Data("\n".utf8))
