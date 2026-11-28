import SwiftUI
import UIKit

/// Spike S5 app. Launch arguments drive one run: -dir <state> -code <6 digits> -fileKeys 0|1.
@main
struct S5App: App {
    @State private var log = "starting"
    var body: some Scene {
        WindowGroup {
            if ProcessInfo.processInfo.arguments.contains("-shareTest") {
                ShareTestView()
            } else {
                ScrollView { Text(log).font(.system(.footnote, design: .monospaced)).padding() }
                    .task { log = await Runner.run() }
            }
        }
    }
}

enum Runner {
    static func arg(_ name: String) -> String? {
        let a = ProcessInfo.processInfo.arguments
        guard let i = a.firstIndex(of: "-" + name), i + 1 < a.count else { return nil }
        return a[i + 1]
    }

    static func json(_ s: String) -> [String: Any] {
        (try? JSONSerialization.jsonObject(with: Data(s.utf8))) as? [String: Any] ?? [:]
    }

    static func ipc(_ m: String, _ p: [String: Any] = [:]) -> [String: Any] {
        let d = (try? JSONSerialization.data(withJSONObject: p)) ?? Data("{}".utf8)
        return json(call(method: m, paramsJson: String(decoding: d, as: UTF8.self)))
    }

    static func run() async -> String {
        var r: [String: Any] = [:]
        let dir = arg("dir") ?? NSTemporaryDirectory() + "s5"
        if let lb = arg("loopback") { setenv("UCLIP_TEST_LOOPBACK", lb, 1) }
        r["footprint_before"] = footprint()
        let err = start(profile: "s5sim", stateDir: dir, fileKeys: arg("fileKeys") == "1")
        r["start_error"] = err
        // Ready: the IPC answers status.
        var ready = false
        for _ in 0..<200 {
            if ipc("status")["ok"] != nil { ready = true; break }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        r["ready"] = ready
        r["ready_ms"] = elapsedMs()
        r["footprint_ready"] = footprint()
        r["status"] = ipc("status")["ok"] ?? ipc("status")
        // Named pasteboard write and read (never the general pasteboard).
        let pb = UIPasteboard(name: UIPasteboard.Name("uc-s5-test"), create: true)
        pb?.string = "s5 named pasteboard"
        r["named_pasteboard_ok"] = pb?.string == "s5 named pasteboard"
        if let code = arg("code") {
            let t0 = elapsedMs()
            r["pair"] = ipc("pair.join", ["code": code, "leave_other": true, "test_addrs": arg("pairAddrs") ?? NSNull()])
            r["pair_ms"] = elapsedMs() - t0
            // Wait for the link, then copy from here and wait for the Mac's copy.
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            r["devices"] = ipc("devices.list")["ok"]
            let mine = "from-sim-\(Int(Date().timeIntervalSince1970))"
            r["copied"] = mine
            r["copy"] = ipc("debug.copy", ["text": mine])
            var got: String? = nil
            for _ in 0..<300 {
                if let ok = ipc("debug.clipboard")["ok"] as? [String: Any], let t = ok["text"] as? String,
                   t.hasPrefix("from-mac") { got = t; break }
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
            r["received"] = got
            r["footprint_synced"] = footprint()
        }
        r["total_ms"] = elapsedMs()
        let out = (try? JSONSerialization.data(withJSONObject: r, options: [.prettyPrinted, .sortedKeys])) ?? Data()
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? out.write(to: docs.appendingPathComponent("s5.json"))
        let s = String(decoding: out, as: UTF8.self)
        print("S5RESULT " + s.replacingOccurrences(of: "\n", with: " "))
        return s
    }
}

/// Opens the Share sheet with a short text, so a UI test can pick the extension.
struct ShareTestView: View {
    @State private var show = false
    var body: some View {
        Button("Share") { show = true }
            .sheet(isPresented: $show) { ActivityView(items: ["s5 share test"]) }
    }
}

struct ActivityView: UIViewControllerRepresentable {
    let items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}
