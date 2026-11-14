//! `uc`: the Universal Clipboard command line. See the crate docs and `uc help`.

use clap::Parser;

fn main() {
    let cli = uc_cli::Cli::parse();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("uc: {e}");
            std::process::exit(2);
        }
    };
    match rt.block_on(uc_cli::run(cli)) {
        Ok(()) => {}
        Err(uc_cli::Fail(msg, code)) => {
            eprintln!("{msg}");
            std::process::exit(code);
        }
    }
}
