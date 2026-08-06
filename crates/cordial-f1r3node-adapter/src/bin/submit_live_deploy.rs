use anyhow::{Context, Result};
use casper::rust::util::construct_deploy;
use clap::Parser;
use models::casper::v1::{deploy_response, deploy_service_client::DeployServiceClient};
use models::rust::casper::protocol::casper_message::DeployData as ModelDeployData;
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

    #[arg(long, default_value_t = 0)]
    valid_after_block_number: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let endpoint = Endpoint::from_shared(args.grpc_url.clone())
        .context("invalid grpc url for deploy submitter")?;
    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("failed to connect to {}", args.grpc_url))?;
    let mut client = DeployServiceClient::new(channel);

    let mut valid_after_block_number = args.valid_after_block_number;
    if valid_after_block_number == 0
        && let Ok(lfb_resp) = client
            .last_finalized_block(models::casper::LastFinalizedBlockQuery {})
            .await
        && let inner = lfb_resp.into_inner()
        && let Some(models::casper::v1::last_finalized_block_response::Message::BlockInfo(bi)) =
            inner.message
        && let Some(light) = bi.block_info
    {
        valid_after_block_number = light.block_number;
    }

    let signed = construct_deploy::source_deploy_now_full(
        args.term,
        Some(args.phlo_limit),
        Some(args.phlo_price),
        None,
        Some(valid_after_block_number),
        Some(args.shard_id.clone()),
    )
    .context("failed to construct valid secp256k1-signed deploy")?;
    let request = ModelDeployData::to_proto(signed);

    println!("Submitting live deploy");
    println!("=====================");
    println!("gRPC URL:    {}", args.grpc_url);
    println!("timestamp:   {}", request.timestamp);
    println!("shard id:    {}", request.shard_id);
    println!("valid after: {}", valid_after_block_number);
    println!("term:        {}", request.term);
    println!("sig algo:    {}", request.sig_algorithm);
    println!("language:    {}", args.language);

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
