import UIKit

/// Spike S5 Share Extension: starts the Rust core and reports time and memory.
final class ShareViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        let before = footprint()
        let err = start(profile: "s5ext", stateDir: NSTemporaryDirectory() + "s5ext", fileKeys: true)
        let label = UILabel(frame: view.bounds)
        label.numberOfLines = 0
        label.text = "start: \(err.isEmpty ? "ok" : err)\nbefore \(before / 1_048_576) MB, after \(footprint() / 1_048_576) MB, \(elapsedMs()) ms"
        view.addSubview(label)
        label.accessibilityIdentifier = "s5ext"
        NSLog("S5EXT start=%@ before=%llu after=%llu ms=%llu", err.isEmpty ? "ok" : err, before, footprint(), elapsedMs())
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            NSLog("S5EXT after1s=%llu", footprint())
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
            self.extensionContext?.completeRequest(returningItems: nil)
        }
    }
}
