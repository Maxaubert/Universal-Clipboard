//! File copies between two real agents on loopback (file sync spec 7, E2E part of plan step
//! 5): `uc copy` of a folder and a file, an automatic copy, the over-limit note, keep-3 in the
//! files folder, and no file names in logs. Fake clipboards; the files are real, in temp
//! folders.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::*;
use serde_json::{Value, json};

/// Bytes that differ along the file, so a misplaced chunk is caught.
fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u32).wrapping_mul(2654435761).to_le_bytes()[1] ^ seed)
        .collect()
}

fn files_on(a: &Agent) -> Vec<PathBuf> {
    a.call("debug.clipboard", json!({}))["files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect()
}

fn last_activity(a: &Agent) -> Value {
    a.uc_json(&["status"])["activity"][0].clone()
}

/// Received copies in an agent's files folder (`<state>/files` for test profiles).
fn received_folders(a: &Agent) -> Vec<String> {
    std::fs::read_dir(a.dir.join("files"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn files_reach_the_other_agent() {
    let mut a = Agent::new(91);
    let mut b = Agent::new(92);
    a.start();
    b.start();
    a.uc_json(&["settings", "set", "device-name", "Alpha"]);
    b.uc_json(&["settings", "set", "device-name", "Bravo"]);
    pair(&a, &mut b);
    assert!(
        until(Duration::from_secs(10), || {
            a.uc_json(&["status"])["connected"] == 1 && b.uc_json(&["status"])["connected"] == 1
        }),
        "not connected:\n{}",
        a.log()
    );
    assert_eq!(a.uc_json(&["settings"])["sync-files"]["value"], "on");
    assert_eq!(a.uc_json(&["settings"])["file-limit-mb"]["value"], "100");

    // A folder with a nested file, an empty folder, and a loose file, picked with `uc copy`.
    let src = tempfile::tempdir().unwrap();
    let photos = src.path().join("Secret Photos");
    std::fs::create_dir_all(photos.join("trip")).unwrap();
    std::fs::create_dir_all(photos.join("empty")).unwrap();
    let big = pattern(3 << 20, 7);
    std::fs::write(photos.join("trip").join("beach.bin"), &big).unwrap();
    std::fs::write(photos.join("note-zebra.txt"), b"hello").unwrap();
    let loose = src.path().join("report-walrus.pdf");
    std::fs::write(&loose, pattern(1000, 3)).unwrap();
    let missing = src.path().join("nope");
    let o = a.uc(&["copy", missing.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("There is no file or folder"));

    let r = a.uc_json(&["copy", photos.to_str().unwrap(), loose.to_str().unwrap()]);
    assert_eq!(r["preparing"], true);
    assert!(
        until(Duration::from_secs(20), || files_on(&b).len() == 2),
        "files did not arrive:\nA:\n{}\nB:\n{}",
        a.log(),
        b.log()
    );
    let got = files_on(&b);
    let root = b.dir.join("files");
    for p in &got {
        assert!(p.is_absolute() && p.starts_with(&root), "{}", p.display());
    }
    let dir = got.iter().find(|p| p.is_dir()).expect("the folder");
    let file = got.iter().find(|p| p.is_file()).expect("the file");
    assert_eq!(dir.file_name().unwrap(), "Secret Photos");
    assert_eq!(
        std::fs::read(dir.join("trip").join("beach.bin")).unwrap(),
        big
    );
    assert_eq!(std::fs::read(dir.join("note-zebra.txt")).unwrap(), b"hello");
    assert!(dir.join("empty").is_dir());
    assert_eq!(std::fs::read(file).unwrap(), pattern(1000, 3));
    // No partial folder is left.
    assert!(received_folders(&b).iter().all(|n| !n.ends_with(".part")));

    // The picked items are on A's own clipboard too (written with our marker).
    let mine = files_on(&a);
    assert_eq!(mine, vec![photos.clone(), loose.clone()]);

    // Last activity: counts and sizes only.
    let ra = last_activity(&b);
    assert_eq!(ra["direction"], "received");
    assert_eq!(ra["device"], "Alpha");
    assert_eq!(ra["kind"], "3 files, 3 MB");
    assert!(until(
        Duration::from_secs(5),
        || last_activity(&a)["direction"] == "sent"
    ));
    assert_eq!(last_activity(&a)["kind"], "3 files, 3 MB");
    assert!(until(Duration::from_secs(5), || {
        a.uc_json(&["status"])["transfer"].is_null() && b.uc_json(&["status"])["transfer"].is_null()
    }));
    assert!(b.uc_json(&["status"])["last_transfer_error"].is_null());
    assert_eq!(a.uc_json(&["transfer", "stop"])["stopped"], false);

    // An automatic copy over the limit is not sent, with a note on this device.
    a.uc_json(&["settings", "set", "file-limit-mb", "1"]);
    let over = src.path().join("over.bin");
    std::fs::write(&over, pattern(2 << 20, 1)).unwrap();
    a.call("debug.copy_files", json!({ "paths": [over] }));
    assert!(
        until(Duration::from_secs(10), || {
            last_activity(&a)["note"]
                .as_str()
                .is_some_and(|n| n.contains("Files over 1 MB are not sent automatically"))
        }),
        "{}",
        a.uc_json(&["status"])
    );

    // Small automatic copies sync; the files folder keeps the last 3.
    for i in 0..3 {
        let f = src.path().join(format!("small-{i}.txt"));
        std::fs::write(&f, format!("small {i}")).unwrap();
        a.call("debug.copy_files", json!({ "paths": [f] }));
        assert!(
            until(Duration::from_secs(15), || {
                files_on(&b)
                    .first()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .as_deref()
                    == Some(format!("small {i}").as_str())
            }),
            "small copy {i} did not arrive:\nA:\n{}\nB:\n{}",
            a.log(),
            b.log()
        );
    }
    assert!(
        until(Duration::from_secs(5), || received_folders(&b).len() == 3),
        "{:?}",
        received_folders(&b)
    );

    // Sync files off on the receiver: a new copy is not fetched.
    b.uc_json(&["settings", "set", "sync-files", "off"]);
    let f = src.path().join("while-off.txt");
    std::fs::write(&f, b"off").unwrap();
    a.call("debug.copy_files", json!({ "paths": [f] }));
    assert!(until(Duration::from_secs(5), || last_activity(&a)["kind"]
        == "1 file, 3 bytes"));
    std::thread::sleep(Duration::from_millis(800));
    assert!(files_on(&b).iter().all(|p| !p.ends_with("while-off.txt")));
    // And `uc copy` there says why it cannot.
    let o = b.uc(&["copy", loose.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("sync-files on"));

    // Logs and the report hold no file or folder names.
    for x in [&a, &b] {
        let log = x.log();
        for name in [
            "Secret Photos",
            "note-zebra",
            "report-walrus",
            "beach.bin",
            "small-",
        ] {
            assert!(!log.contains(name), "agent {} logged {name}", x.n);
        }
        let r = x.uc_json(&["diagnostics"])["report"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!r.contains("walrus") && !r.contains("zebra"));
    }
}
