use std::net::SocketAddr;
use std::sync::Arc;

use cordial_f1r3node_adapter::live_deploy_proxy::LiveDeployProxy;
use models::casper::v1::{
    deploy_response, deploy_service_server::{DeployService, DeployServiceServer},
    status_response, BlockInfoResponse, BlockResponse, BondStatusResponse,
    ContinuationAtNameResponse, DeployResponse, EventInfoResponse, ExploratoryDeployResponse,
    FindDeployResponse, IsFinalizedResponse, LastFinalizedBlockResponse,
    ListeningNameDataResponse, MachineVerifyResponse, PrivateNamePreviewResponse, RhoDataResponse,
    StatusResponse, VisualizeBlocksResponse,
};
use models::casper::{
    BlockQuery, BlocksQuery, BlocksQueryByHeight, BondStatusQuery, ContinuationAtNameQuery,
    DataAtNameByBlockQuery, DataAtNameQuery, DeployDataProto, ExploratoryDeployQuery,
    FindDeployQuery, IsFinalizedQuery, LastFinalizedBlockQuery, MachineVerifyQuery,
    PrivateNamePreviewQuery, ReportQuery, Status, VisualizeDagQuery,
};
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Server;

#[derive(Clone, Default)]
struct RecordingUpstream {
    deploys: Arc<Mutex<Vec<DeployDataProto>>>,
}

impl RecordingUpstream {
    async fn recorded_deploys(&self) -> Vec<DeployDataProto> {
        self.deploys.lock().await.clone()
    }
}

#[tonic::async_trait]
impl DeployService for RecordingUpstream {
    type visualizeDagStream = ReceiverStream<Result<VisualizeBlocksResponse, tonic::Status>>;
    type showMainChainStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;
    type getBlocksStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;
    type getBlocksByHeightsStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;

    async fn do_deploy(
        &self,
        request: tonic::Request<DeployDataProto>,
    ) -> Result<tonic::Response<DeployResponse>, tonic::Status> {
        self.deploys.lock().await.push(request.into_inner());
        Ok(tonic::Response::new(DeployResponse {
            message: Some(deploy_response::Message::Result("forwarded".to_string())),
        }))
    }

    async fn get_block(
        &self,
        _request: tonic::Request<BlockQuery>,
    ) -> Result<tonic::Response<BlockResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("getBlock"))
    }

    async fn visualize_dag(
        &self,
        _request: tonic::Request<VisualizeDagQuery>,
    ) -> Result<tonic::Response<Self::visualizeDagStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("visualizeDag"))
    }

    async fn machine_verifiable_dag(
        &self,
        _request: tonic::Request<MachineVerifyQuery>,
    ) -> Result<tonic::Response<MachineVerifyResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("machineVerifiableDag"))
    }

    async fn show_main_chain(
        &self,
        _request: tonic::Request<BlocksQuery>,
    ) -> Result<tonic::Response<Self::showMainChainStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("showMainChain"))
    }

    async fn get_blocks(
        &self,
        _request: tonic::Request<BlocksQuery>,
    ) -> Result<tonic::Response<Self::getBlocksStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("getBlocks"))
    }

    async fn listen_for_data_at_name(
        &self,
        _request: tonic::Request<DataAtNameQuery>,
    ) -> Result<tonic::Response<ListeningNameDataResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("listenForDataAtName"))
    }

    async fn get_data_at_name(
        &self,
        _request: tonic::Request<DataAtNameByBlockQuery>,
    ) -> Result<tonic::Response<RhoDataResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("getDataAtName"))
    }

    async fn listen_for_continuation_at_name(
        &self,
        _request: tonic::Request<ContinuationAtNameQuery>,
    ) -> Result<tonic::Response<ContinuationAtNameResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("listenForContinuationAtName"))
    }

    async fn find_deploy(
        &self,
        _request: tonic::Request<FindDeployQuery>,
    ) -> Result<tonic::Response<FindDeployResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("findDeploy"))
    }

    async fn preview_private_names(
        &self,
        _request: tonic::Request<PrivateNamePreviewQuery>,
    ) -> Result<tonic::Response<PrivateNamePreviewResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("previewPrivateNames"))
    }

    async fn last_finalized_block(
        &self,
        _request: tonic::Request<LastFinalizedBlockQuery>,
    ) -> Result<tonic::Response<LastFinalizedBlockResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("lastFinalizedBlock"))
    }

    async fn is_finalized(
        &self,
        _request: tonic::Request<IsFinalizedQuery>,
    ) -> Result<tonic::Response<IsFinalizedResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("isFinalized"))
    }

    async fn bond_status(
        &self,
        _request: tonic::Request<BondStatusQuery>,
    ) -> Result<tonic::Response<BondStatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("bondStatus"))
    }

    async fn exploratory_deploy(
        &self,
        _request: tonic::Request<ExploratoryDeployQuery>,
    ) -> Result<tonic::Response<ExploratoryDeployResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("exploratoryDeploy"))
    }

    async fn get_blocks_by_heights(
        &self,
        _request: tonic::Request<BlocksQueryByHeight>,
    ) -> Result<tonic::Response<Self::getBlocksByHeightsStream>, tonic::Status> {
        Err(tonic::Status::unimplemented("getBlocksByHeights"))
    }

    async fn get_event_by_hash(
        &self,
        _request: tonic::Request<ReportQuery>,
    ) -> Result<tonic::Response<EventInfoResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("getEventByHash"))
    }

    async fn status(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        Ok(tonic::Response::new(StatusResponse {
            message: Some(status_response::Message::Status(Status {
                version: None,
                address: "proxy-upstream".to_string(),
                network_id: "standalone-dev".to_string(),
                shard_id: "root".to_string(),
                peers: 0,
                nodes: 0,
                min_phlo_price: 1,
                peer_list: vec![],
            })),
        }))
    }
}

async fn spawn_upstream(service: RecordingUpstream) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        Server::builder()
            .add_service(DeployServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    addr
}

fn sample_proto(sig_byte: u8) -> DeployDataProto {
    DeployDataProto {
        deployer: vec![0x11; 33].into(),
        term: "@0!(\"hello\")".to_string(),
        timestamp: 1234,
        sig: vec![sig_byte; 64].into(),
        sig_algorithm: "secp256k1".to_string(),
        phlo_price: 1,
        phlo_limit: 100_000,
        valid_after_block_number: 0,
        shard_id: "root".to_string(),
        language: "rholang".to_string(),
        expiration_timestamp: 0,
    }
}

#[tokio::test]
async fn proxy_observes_and_forwards_do_deploy() {
    let upstream = RecordingUpstream::default();
    let addr = spawn_upstream(upstream.clone()).await;
    let proxy = LiveDeployProxy::connect(format!("http://{addr}"))
        .await
        .unwrap();

    let req = sample_proto(9);
    let response = proxy
        .do_deploy(tonic::Request::new(req.clone()))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        response.message,
        Some(deploy_response::Message::Result("forwarded".to_string()))
    );

    let observed = proxy.observed_deploys().await;
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].signature, req.sig);

    let forwarded = upstream.recorded_deploys().await;
    assert_eq!(forwarded, vec![req]);
}

#[tokio::test]
async fn proxy_preserves_status_method_name_and_passthrough() {
    let upstream = RecordingUpstream::default();
    let addr = spawn_upstream(upstream).await;
    let proxy = LiveDeployProxy::connect(format!("http://{addr}"))
        .await
        .unwrap();

    let response = proxy.status(tonic::Request::new(())).await.unwrap().into_inner();
    match response.message {
        Some(status_response::Message::Status(status)) => {
            assert_eq!(status.address, "proxy-upstream");
            assert_eq!(status.shard_id, "root");
        }
        other => panic!("unexpected status response: {other:?}"),
    }
}
