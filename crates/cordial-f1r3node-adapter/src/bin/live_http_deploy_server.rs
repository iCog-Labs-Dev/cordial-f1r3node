use anyhow::{Context, Result};
use clap::Parser;
use cordial_f1r3node_adapter::http_deploy_ingress::{
    HttpDeployIngressState, serve_http_deploy_ingress,
};
use cordial_f1r3node_adapter::live_deploy_ingress::LiveDeployIngress;

#[derive(Debug, Parser)]
#[command(name = "live_http_deploy_server")]
#[command(
    about = "Cordial HTTP deploy ingress server that accepts deploy requests and observes them"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:40412")]
    bind_addr: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr: std::net::SocketAddr = args
        .bind_addr
        .parse()
        .with_context(|| format!("invalid bind address {}", args.bind_addr))?;

    println!("Cordial HTTP deploy ingress server");
    println!("==================================");
    println!("Bind address: {}", args.bind_addr);
    println!();
    println!("POST /api/deploy  — observe a deploy (JSON HttpDeployRequest)");
    println!("POST /api/v1/deploy — same handler (versioned path)");

    let state = HttpDeployIngressState::new(LiveDeployIngress::new());
    serve_http_deploy_ingress(state, addr)
        .await
        .context("HTTP deploy ingress server exited with error")?;

    Ok(())
}
