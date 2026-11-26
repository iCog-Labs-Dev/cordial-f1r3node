//! Generate fresh signed HTTP deploy requests as JSONL; does not submit them.
//! Uses the host's funded development key, never a production credential.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use crypto::rust::private_key::PrivateKey;
use std::io::{BufWriter, Write};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 100)]
    count: u32,
    #[arg(long, default_value_t = 0)]
    valid_after_block_number: i64,
    #[arg(long, default_value = "root")]
    shard_id: String,
    #[arg(long, default_value_t = 100_000)]
    phlo_limit: i64,
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.count > 0, "count must be positive");
    ensure!(
        args.valid_after_block_number >= 0,
        "valid-after-block-number must be nonnegative"
    );
    ensure!(args.phlo_limit > 0, "phlo-limit must be positive");
    let start = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let signer = PrivateKey::from_bytes(
        &hex::decode("a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76")
            .context("development signer")?,
    );
    let mut output = BufWriter::new(std::io::stdout().lock());
    for i in 0..args.count {
        let timestamp = start
            .checked_add(i64::from(i))
            .context("timestamp overflow")?;
        let signed = casper::rust::util::construct_deploy::source_deploy(
            format!("@0!(\"load-{start}-{i}\")"),
            timestamp,
            Some(args.phlo_limit),
            Some(1),
            Some(signer.clone()),
            Some(args.valid_after_block_number),
            Some(args.shard_id.clone()),
        )
        .context("sign development deploy")?;
        let request = serde_json::json!({
            "data": signed.data,
            "deployer": hex::encode(&signed.pk.bytes),
            "signature": hex::encode(&signed.sig),
            "sigAlgorithm": "secp256k1"
        });
        serde_json::to_writer(&mut output, &request)?;
        writeln!(output)?;
    }
    output.flush()?;
    Ok(())
}
