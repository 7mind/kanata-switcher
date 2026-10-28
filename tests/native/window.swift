import AppKit

// A real app with a bundle identifier and window. Commands enter through stdin;
// all AppKit work stays on the main thread. No AppleScript or Accessibility grants.
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let window = NSWindow(contentRect: NSRect(x: 80, y: 80, width: 400, height: 200),
                      styleMask: [.titled, .closable], backing: .buffered, defer: false)
window.title = "Native focus fixture"
window.makeKeyAndOrderFront(nil)
DispatchQueue.global().async {
    while let command = readLine() {
        DispatchQueue.main.async {
            if command == "focus" {
                window.makeKeyAndOrderFront(nil)
                app.activate(ignoringOtherApps: true)
                func confirmFocus(attempts: Int) {
                    if NSWorkspace.shared.frontmostApplication?.processIdentifier == ProcessInfo.processInfo.processIdentifier {
                        print("FOCUSED")
                        fflush(stdout)
                    } else if attempts > 0 {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { confirmFocus(attempts: attempts - 1) }
                    } else {
                        print("ERROR focus denied")
                        fflush(stdout)
                    }
                }
                confirmFocus(attempts: 60)
            } else if command == "quit" {
                app.terminate(nil)
            }
        }
    }
    DispatchQueue.main.async { app.terminate(nil) }
}
print("READY")
fflush(stdout)
app.run()
