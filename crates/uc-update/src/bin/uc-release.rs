//! `uc-release`: updater keys, signatures and `latest.json` for `release.yml` (plan 8.4).
//!
//! The secret key comes from `--key FILE` or the environment variable `UCLIP_UPDATER_KEY`
//! (the GitHub environment `release` sets it). It is never printed.
//!
//! ```text
//! uc-release keygen FILE                 write a new secret key to FILE (refuses to overwrite), print the public key
//! uc-release pubkey                      print the public key of the secret key
//! uc-release manifest --version V --rollout N --base-url URL --out latest.json TARGET:KIND:PATH...
//! uc-release rollout --in latest.json --pct N --out latest.json
//! uc-release verify --pub KEY latest.json [DIR]   check latest.json (and the files in DIR)
//! ```

use std::path::{Path, PathBuf};
use std::process::exit;

use uc_update::manifest::{FileEntry, Kind, Manifest, SCHEMA, Signed};
use uc_update::sign::SecretKey;

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("uc-release: {msg}");
    exit(1)
}

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    if i + 1 >= args.len() {
        die(format!("{flag} needs a value"));
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

fn secret(args: &mut Vec<String>) -> SecretKey {
    let text = match take(args, "--key") {
        Some(f) => std::fs::read_to_string(&f).unwrap_or_else(|e| die(format!("{f}: {e}"))),
        None => std::env::var("UCLIP_UPDATER_KEY")
            .unwrap_or_else(|_| die("no key: use --key FILE or UCLIP_UPDATER_KEY")),
    };
    SecretKey::from_text(&text).unwrap_or_else(|e| die(e))
}

fn today() -> String {
    // Days since 1970 to a civil date (no time crate needed).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        die("usage: keygen | pubkey | manifest | rollout | verify (see the source header)");
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "keygen" => {
            let Some(f) = args.first() else {
                die("keygen FILE")
            };
            let p = Path::new(f);
            if p.exists() {
                die(format!("{f} exists; not overwriting a key"));
            }
            let k = SecretKey::generate();
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(p, k.to_text().as_bytes()).unwrap_or_else(|e| die(e));
            println!("{}", k.public_b64());
        }
        "pubkey" => println!("{}", secret(&mut args).public_b64()),
        "manifest" => {
            let key = secret(&mut args);
            let version = take(&mut args, "--version").unwrap_or_else(|| die("--version"));
            let rollout: u8 = take(&mut args, "--rollout")
                .unwrap_or_else(|| "10".into())
                .parse()
                .unwrap_or_else(|_| die("--rollout 0..100"));
            let base = take(&mut args, "--base-url").unwrap_or_else(|| die("--base-url"));
            let out = take(&mut args, "--out").unwrap_or_else(|| "latest.json".into());
            if uc_update::manifest::parse_version(&version).is_none() || rollout > 100 {
                die("bad --version or --rollout");
            }
            let mut files = Vec::new();
            for spec in &args {
                let mut it = spec.splitn(3, ':');
                let (Some(target), Some(kind), Some(path)) = (it.next(), it.next(), it.next())
                else {
                    die(format!("{spec}: want TARGET:KIND:PATH"));
                };
                let kind = match kind {
                    "nsis" => Kind::Nsis,
                    "tar" => Kind::Tar,
                    _ => die(format!("{kind}: kind is nsis or tar")),
                };
                let path = PathBuf::from(path);
                let data = std::fs::read(&path).unwrap_or_else(|e| die(format!("{spec}: {e}")));
                let name = path
                    .file_name()
                    .unwrap_or_else(|| die("no file name"))
                    .to_string_lossy()
                    .to_string();
                let url = format!("{}/{}", base.trim_end_matches('/'), name);
                files.push(
                    FileEntry::make(target, kind, &name, &url, &version, &data, &key)
                        .unwrap_or_else(|e| die(e)),
                );
            }
            let m = Manifest {
                schema: SCHEMA,
                version,
                released: today(),
                rollout,
                files,
            };
            let s = Signed::sign(&m, &key).unwrap_or_else(|e| die(e));
            std::fs::write(&out, serde_json::to_vec_pretty(&s).unwrap()).unwrap_or_else(|e| die(e));
            println!("wrote {out}: {} at {}%", m.version, m.rollout);
        }
        "rollout" => {
            let key = secret(&mut args);
            let input = take(&mut args, "--in").unwrap_or_else(|| die("--in"));
            let pct: u8 = take(&mut args, "--pct")
                .unwrap_or_else(|| die("--pct"))
                .parse()
                .unwrap_or_else(|_| die("--pct 0..100"));
            let out = take(&mut args, "--out").unwrap_or_else(|| input.clone());
            let bytes = std::fs::read(&input).unwrap_or_else(|e| die(e));
            let mut m = Signed::parse(&bytes)
                .and_then(|s| s.open(&key.public_b64()))
                .unwrap_or_else(|e| die(e));
            if pct > 100 {
                die("--pct 0..100");
            }
            m.rollout = pct;
            let s = Signed::sign(&m, &key).unwrap_or_else(|e| die(e));
            std::fs::write(&out, serde_json::to_vec_pretty(&s).unwrap()).unwrap_or_else(|e| die(e));
            println!("{} now at {pct}%", m.version);
        }
        "verify" => {
            let public = take(&mut args, "--pub")
                .unwrap_or_else(|| uc_update::UPDATER_PUBLIC_KEY.to_string());
            let Some(input) = args.first() else {
                die("verify latest.json [DIR]")
            };
            let bytes = std::fs::read(input).unwrap_or_else(|e| die(e));
            let m = Signed::parse(&bytes)
                .and_then(|s| s.open(&public))
                .unwrap_or_else(|e| die(e));
            println!(
                "latest.json: {} at {}%, signature good",
                m.version, m.rollout
            );
            if let Some(dir) = args.get(1) {
                for f in &m.files {
                    let data = std::fs::read(Path::new(dir).join(&f.name))
                        .unwrap_or_else(|e| die(format!("{}: {e}", f.name)));
                    f.verify(&data, &m.version, &public)
                        .unwrap_or_else(|e| die(format!("{}: {e}", f.name)));
                    println!("{} ({}): good", f.name, f.target);
                }
            }
        }
        _ => die(format!("unknown command {cmd}")),
    }
}
