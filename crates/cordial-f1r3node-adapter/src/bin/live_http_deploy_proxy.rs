use anyhow::{Context, Result};
use clap::Parser;
use cordial_f1r3node_adapter::live_http_deploy_proxy::{
    LiveHttpDeployProxy, serve_live_http_deploy_proxy,
};

#[derive(Debug, Parser)]
#[command(name = "live_http_deploy_proxy")]
#[command(
    about = "HTTP deploy proxy that observes and forwards deploy requests to a real f1r3node HTTP API"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:40413")]
    bind_addr: String,

    #[arg(long, default_value = "http://127.0.0.1:40403")]
    upstream_http_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let proxy = LiveHttpDeployProxy::new(args.upstream_http_url.clone());
    let addr = args
        .bind_addr
        .parse()
        .with_context(|| format!("invalid bind address {}", args.bind_addr))?;

    println!("Cordial live HTTP deploy proxy");
    println!("==============================");
    println!("Bind address:      {}", args.bind_addr);
    println!("Upstream HTTP URL: {}", args.upstream_http_url);

    serve_live_http_deploy_proxy(proxy, addr)
        .await
        .context("HTTP deploy proxy server exited with error")?;

    Ok(())
}
