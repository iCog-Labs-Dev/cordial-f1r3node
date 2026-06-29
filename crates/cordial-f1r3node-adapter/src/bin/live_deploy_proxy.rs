use anyhow::{Context, Result};
use clap::Parser;
use cordial_f1r3node_adapter::live_deploy_proxy::LiveDeployProxy;
use models::casper::v1::deploy_service_server::DeployServiceServer;
use tonic::transport::Server;

#[derive(Debug, Parser)]
#[command(name = "live_deploy_proxy")]
#[command(about = "External Cordial deploy proxy that observes doDeploy and forwards upstream")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:40411")]
    bind_addr: String,

    #[arg(long, default_value = "http://127.0.0.1:40401")]
    upstream_grpc_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let proxy = LiveDeployProxy::connect(args.upstream_grpc_url.clone())
        .await
        .with_context(|| format!("failed to connect to upstream {}", args.upstream_grpc_url))?;
    let addr = args
        .bind_addr
        .parse()
        .with_context(|| format!("invalid bind address {}", args.bind_addr))?;

    println!("Cordial live deploy proxy");
    println!("=========================");
    println!("Bind address:      {}", args.bind_addr);
    println!("Upstream gRPC URL: {}", args.upstream_grpc_url);

    Server::builder()
        .add_service(DeployServiceServer::new(proxy))
        .serve(addr)
        .await
        .context("deploy proxy server exited with error")?;

    Ok(())
}
