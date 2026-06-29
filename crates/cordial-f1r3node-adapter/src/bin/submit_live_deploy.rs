use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use models::casper::DeployDataProto;
use models::casper::v1::{deploy_response, deploy_service_client::DeployServiceClient};
use tonic::transport::Endpoint;

#[derive(Debug, Parser)]
#[command(name = "submit_live_deploy")]
#[command(about = "Submit a test deploy to a live f1r3node DeployService endpoint")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:40411")]
    grpc_url: String,

    #[arg(long, default_value = "@0!(\"hello cordial\")")]
    term: String,

    #[arg(long, default_value_t = 1)]
    phlo_price: i64,

    #[arg(long, default_value_t = 100_000)]
    phlo_limit: i64,

    #[arg(long, default_value = "root")]
    shard_id: String,

    #[arg(long, default_value = "rholang")]
    language: String,

    #[arg(
        long,
        default_value = "020101010101010101010101010101010101010101010101010101010101010101"
    )]
    deployer_hex: String,

    #[arg(
        long,
        default_value = "07070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707070707"
    )]
    signature_hex: String,

    #[arg(long, default_value = "secp256k1")]
    sig_algorithm: String,

    #[arg(long)]
    timestamp_ms: Option<i64>,

    #[arg(long, default_value_t = 0)]
    valid_after_block_number: i64,

    #[arg(long, default_value_t = 0)]
    expiration_timestamp: i64,
}

fn now_timestamp_ms() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(elapsed.as_millis()).context("timestamp overflow while converting to i64")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let endpoint =
        Endpoint::from_shared(args.grpc_url.clone()).context("invalid grpc url for deploy submitter")?;
    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("failed to connect to {}", args.grpc_url))?;
    let mut client = DeployServiceClient::new(channel);

    let deployer = hex::decode(&args.deployer_hex).context("failed to decode --deployer-hex")?;
    let sig = hex::decode(&args.signature_hex).context("failed to decode --signature-hex")?;
    let timestamp = match args.timestamp_ms {
        Some(ts) => ts,
        None => now_timestamp_ms()?,
    };

    let request = DeployDataProto {
        deployer: deployer.into(),
        term: args.term,
        timestamp,
        sig: sig.into(),
        sig_algorithm: args.sig_algorithm,
        phlo_price: args.phlo_price,
        phlo_limit: args.phlo_limit,
        valid_after_block_number: args.valid_after_block_number,
        shard_id: args.shard_id,
        language: args.language,
        expiration_timestamp: args.expiration_timestamp,
    };

    println!("Submitting live deploy");
    println!("=====================");
    println!("gRPC URL:    {}", args.grpc_url);
    println!("timestamp:   {}", timestamp);
    println!("shard id:    {}", request.shard_id);
    println!("term:        {}", request.term);

    let response = client
        .do_deploy(request)
        .await
        .context("doDeploy request failed")?
        .into_inner();

    match response.message {
        Some(deploy_response::Message::Result(result)) => {
            println!("Deploy accepted: {result}");
        }
        Some(deploy_response::Message::Error(err)) => {
            println!("Deploy rejected: {:?}", err);
        }
        None => {
            println!("Deploy response missing message");
        }
    }

    Ok(())
}
