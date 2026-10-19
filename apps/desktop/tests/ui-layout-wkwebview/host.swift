// Isolated copied-React layout host, not a Tauri production acceptance fixture.
import Cocoa
import WebKit
setbuf(stdout, nil)
let out = CommandLine.arguments[1]
// One fixed synthetic WAV for paused metadata/seek only; not the production media protocol.
class SyntheticMedia: NSObject, WKURLSchemeHandler {
    let bytes = try! Data(contentsOf: URL(fileURLWithPath: out + "/synthetic.wav"))
    var requests = 0
    func webView(_ webView: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        guard let url = urlSchemeTask.request.url, url.absoluteString == "omm-media://localhost/fixture_media_grant_000001" else {
            urlSchemeTask.didFailWithError(NSError(domain: "synthetic-media", code: 1))
            return
        }
        requests += 1
        urlSchemeTask.didReceive(URLResponse(url: url, mimeType: "audio/wav", expectedContentLength: bytes.count, textEncodingName: nil))
        urlSchemeTask.didReceive(bytes)
        urlSchemeTask.didFinish()
    }
    func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {}
}
let media = SyntheticMedia()
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
class Probe: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    var web: WKWebView!
    var window: NSWindow!
    var results: [[String: Any]] = []
    var finished = false
    func save() {
        try! JSONSerialization.data(withJSONObject: results, options: [.prettyPrinted, .sortedKeys])
            .write(to: URL(fileURLWithPath: out + "/layout-results.json"))
    }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let value = message.body as? [String: Any] else { return }
        if value["done"] as? Bool == true {
            finished = true
            save()
            NSApp.terminate(nil)
            return
        }
        if value["resize"] as? Bool == true, let width = value["width"] as? Int, let height = value["height"] as? Int {
            window.setContentSize(NSSize(width: width, height: height))
            web.frame = NSRect(x: 0, y: 0, width: width, height: height)
            web.evaluateJavaScript("window.__ack?.()")
            return
        }
        var observed = value
        observed["nativeKeyWindow"] = window.isKeyWindow
        observed["nativeAppActive"] = NSApp.isActive
        observed["syntheticMediaRequests"] = media.requests
        results.append(observed)
        save()
        if value["capture"] as? Bool == true, let name = value["name"] as? String {
            web.takeSnapshot(with: nil) { image, _ in
                if let image = image, let tiff = image.tiffRepresentation,
                   let bitmap = NSBitmapImageRep(data: tiff),
                   let png = bitmap.representation(using: .png, properties: [:]) {
                    try? png.write(to: URL(fileURLWithPath: out + "/" + name + ".png"))
                }
                self.web.evaluateJavaScript("window.__ack?.()")
            }
        } else {
            web.evaluateJavaScript("window.__ack?.()")
        }
    }
    func webView(_ view: WKWebView, didFinish navigation: WKNavigation!) {
        let script = try! String(contentsOfFile: out + "/driver.js", encoding: .utf8)
        web.evaluateJavaScript(script + "\nvoid 0;") { _, error in
            if let error = error { print("driver evaluation:", error) }
        }
    }
}
let probe = Probe()
let config = WKWebViewConfiguration()
config.websiteDataStore = .nonPersistent()
config.setURLSchemeHandler(media, forURLScheme: "omm-media")
config.userContentController.add(probe, name: "probe")
probe.web = WKWebView(frame: NSRect(x: 0, y: 0, width: 1280, height: 820), configuration: config)
probe.web.navigationDelegate = probe
probe.window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
probe.window.contentView = probe.web
probe.window.makeKeyAndOrderFront(nil)
probe.web.loadFileURL(URL(fileURLWithPath: out + "/inline.html"), allowingReadAccessTo: URL(fileURLWithPath: out, isDirectory: true))
print("persistent=\(probe.web.configuration.websiteDataStore.isPersistent); copied React; synthetic transport; no hardware playback")
DispatchQueue.main.asyncAfter(deadline: .now() + 360) { print("timeout"); NSApp.terminate(nil) }
app.run()
print("finished=\(probe.finished)")
exit(probe.finished ? 0 : 1)
