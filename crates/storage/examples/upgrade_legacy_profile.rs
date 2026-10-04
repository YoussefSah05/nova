//! Offline, copy-only conversion. Never installs an output into a live profile.
use nova_storage::profile_recovery as legacy_profile;

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        eprintln!("Usage: upgrade_legacy_profile SOURCE_DB NEW_OUTPUT_DB");
        std::process::exit(2);
    }
    match legacy_profile::convert(args[0].as_ref(), args[1].as_ref()).await {
        Ok(()) => println!("Converted known legacy lineage to current schema; preservation, integrity and reopen checks passed. Original unchanged; output not installed."),
        Err(reason) => {
            eprintln!("Conversion refused: {reason}. No output published.");
            std::process::exit(1);
        }
    }
}
