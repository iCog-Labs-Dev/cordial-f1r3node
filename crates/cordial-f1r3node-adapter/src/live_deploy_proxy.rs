use std::sync::Arc;

#[cfg(f1r3node_has_listen_for_data_at_name)]
use models::casper::DataAtNameQuery;
#[cfg(f1r3node_has_deploy_finalization_status)]
use models::casper::DeployFinalizationStatusQuery;
#[cfg(f1r3node_has_deploy_finalization_status)]
use models::casper::v1::DeployFinalizationStatusResponse;
#[cfg(f1r3node_has_listen_for_data_at_name)]
use models::casper::v1::ListeningNameDataResponse;
use models::casper::v1::{
    BlockInfoResponse, BlockResponse, BondStatusResponse, ContinuationAtNameResponse,
    DeployResponse, EventInfoResponse, ExploratoryDeployResponse, FindDeployResponse,
    IsFinalizedResponse, LastFinalizedBlockResponse, MachineVerifyResponse,
    PrivateNamePreviewResponse, RhoDataResponse, StatusResponse, VisualizeBlocksResponse,
    deploy_service_client::DeployServiceClient, deploy_service_server::DeployService,
};
use models::casper::{
    BlockQuery, BlocksQuery, BlocksQueryByHeight, BondStatusQuery, ContinuationAtNameQuery,
    DataAtNameByBlockQuery, DeployDataProto, ExploratoryDeployQuery, FindDeployQuery,
    IsFinalizedQuery, LastFinalizedBlockQuery, MachineVerifyQuery, PrivateNamePreviewQuery,
    ReportQuery, VisualizeDagQuery,
};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};

use crate::live_deploy_ingress::{LiveDeployIngress, ObservedDeploy};

#[derive(Debug)]
pub enum LiveDeployProxyError {
    Transport(tonic::transport::Error),
    Status(tonic::Status),
}

impl std::fmt::Display for LiveDeployProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(err) => {
                write!(f, "failed to connect to upstream deploy service: {err}")
            }
            Self::Status(err) => write!(f, "upstream deploy service call failed: {err}"),
        }
    }
}

impl std::error::Error for LiveDeployProxyError {}

#[derive(Clone)]
pub struct LiveDeployProxy {
    upstream: Arc<Mutex<DeployServiceClient<Channel>>>,
    ingress: Arc<Mutex<LiveDeployIngress>>,
}

impl LiveDeployProxy {
    pub async fn connect(uri: impl Into<String>) -> Result<Self, LiveDeployProxyError> {
        let endpoint =
            Endpoint::from_shared(uri.into()).map_err(LiveDeployProxyError::Transport)?;
        let channel = endpoint
            .connect()
            .await
            .map_err(LiveDeployProxyError::Transport)?;
        Ok(Self::new(DeployServiceClient::new(channel)))
    }

    pub fn new(client: DeployServiceClient<Channel>) -> Self {
        Self {
            upstream: Arc::new(Mutex::new(client)),
            ingress: Arc::new(Mutex::new(LiveDeployIngress::new())),
        }
    }

    /// Share an existing `LiveDeployIngress` with another component
    /// (e.g. a `LiveHttpDeployProxy`) so gRPC- and HTTP-observed deploys
    /// land in one unified staged view.
    pub fn with_shared_ingress(
        client: DeployServiceClient<Channel>,
        ingress: Arc<Mutex<LiveDeployIngress>>,
    ) -> Self {
        Self {
            upstream: Arc::new(Mutex::new(client)),
            ingress,
        }
    }

    pub async fn observed_deploys(&self) -> Vec<ObservedDeploy> {
        self.ingress
            .lock()
            .await
            .staged_deploys()
            .into_iter()
            .cloned()
            .collect()
    }

    pub async fn observe_do_deploy(&self, proto: &DeployDataProto) -> ObservedDeploy {
        self.ingress.lock().await.observe_grpc_proto_deploy(proto)
    }
}

async fn proxy_server_stream<T>(
    mut upstream_stream: tonic::Streaming<T>,
) -> Result<ReceiverStream<Result<T, tonic::Status>>, tonic::Status>
where
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel(128);
    tokio::spawn(async move {
        loop {
            match upstream_stream.message().await {
                Ok(Some(item)) => {
                    if tx.send(Ok(item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    let _ = tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });
    Ok(ReceiverStream::new(rx))
}

#[tonic::async_trait]
impl DeployService for LiveDeployProxy {
    type visualizeDagStream = ReceiverStream<Result<VisualizeBlocksResponse, tonic::Status>>;
    type showMainChainStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;
    type getBlocksStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;
    type getBlocksByHeightsStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;

    async fn do_deploy(
        &self,
        request: tonic::Request<DeployDataProto>,
    ) -> Result<tonic::Response<DeployResponse>, tonic::Status> {
        self.observe_do_deploy(request.get_ref()).await;
        self.upstream.lock().await.do_deploy(request).await
    }

    async fn get_block(
        &self,
        request: tonic::Request<BlockQuery>,
    ) -> Result<tonic::Response<BlockResponse>, tonic::Status> {
        self.upstream.lock().await.get_block(request).await
    }

    async fn visualize_dag(
        &self,
        request: tonic::Request<VisualizeDagQuery>,
    ) -> Result<tonic::Response<Self::visualizeDagStream>, tonic::Status> {
        let response = self.upstream.lock().await.visualize_dag(request).await?;
        let stream = proxy_server_stream(response.into_inner()).await?;
        Ok(tonic::Response::new(stream))
    }

    async fn machine_verifiable_dag(
        &self,
        request: tonic::Request<MachineVerifyQuery>,
    ) -> Result<tonic::Response<MachineVerifyResponse>, tonic::Status> {
        self.upstream
            .lock()
            .await
            .machine_verifiable_dag(request)
            .await
    }

    async fn show_main_chain(
        &self,
        request: tonic::Request<BlocksQuery>,
    ) -> Result<tonic::Response<Self::showMainChainStream>, tonic::Status> {
        let response = self.upstream.lock().await.show_main_chain(request).await?;
        let stream = proxy_server_stream(response.into_inner()).await?;
        Ok(tonic::Response::new(stream))
    }

    async fn get_blocks(
        &self,
        request: tonic::Request<BlocksQuery>,
    ) -> Result<tonic::Response<Self::getBlocksStream>, tonic::Status> {
        let response = self.upstream.lock().await.get_blocks(request).await?;
        let stream = proxy_server_stream(response.into_inner()).await?;
        Ok(tonic::Response::new(stream))
    }

    #[cfg(f1r3node_has_listen_for_data_at_name)]
    async fn listen_for_data_at_name(
        &self,
        request: tonic::Request<DataAtNameQuery>,
    ) -> Result<tonic::Response<ListeningNameDataResponse>, tonic::Status> {
        self.upstream
            .lock()
            .await
            .listen_for_data_at_name(request)
            .await
    }

    #[cfg(f1r3node_has_deploy_finalization_status)]
    async fn deploy_finalization_status(
        &self,
        request: tonic::Request<DeployFinalizationStatusQuery>,
    ) -> Result<tonic::Response<DeployFinalizationStatusResponse>, tonic::Status> {
        self.upstream
            .lock()
            .await
            .deploy_finalization_status(request)
            .await
    }

    async fn get_data_at_name(
        &self,
        request: tonic::Request<DataAtNameByBlockQuery>,
    ) -> Result<tonic::Response<RhoDataResponse>, tonic::Status> {
        self.upstream.lock().await.get_data_at_name(request).await
    }

    async fn listen_for_continuation_at_name(
        &self,
        request: tonic::Request<ContinuationAtNameQuery>,
    ) -> Result<tonic::Response<ContinuationAtNameResponse>, tonic::Status> {
        self.upstream
            .lock()
            .await
            .listen_for_continuation_at_name(request)
            .await
    }

    async fn find_deploy(
        &self,
        request: tonic::Request<FindDeployQuery>,
    ) -> Result<tonic::Response<FindDeployResponse>, tonic::Status> {
        self.upstream.lock().await.find_deploy(request).await
    }

    async fn preview_private_names(
        &self,
        request: tonic::Request<PrivateNamePreviewQuery>,
    ) -> Result<tonic::Response<PrivateNamePreviewResponse>, tonic::Status> {
        self.upstream
            .lock()
            .await
            .preview_private_names(request)
            .await
    }

    async fn last_finalized_block(
        &self,
        request: tonic::Request<LastFinalizedBlockQuery>,
    ) -> Result<tonic::Response<LastFinalizedBlockResponse>, tonic::Status> {
        self.upstream
            .lock()
            .await
            .last_finalized_block(request)
            .await
    }

    async fn is_finalized(
        &self,
        request: tonic::Request<IsFinalizedQuery>,
    ) -> Result<tonic::Response<IsFinalizedResponse>, tonic::Status> {
        self.upstream.lock().await.is_finalized(request).await
    }

    async fn bond_status(
        &self,
        request: tonic::Request<BondStatusQuery>,
    ) -> Result<tonic::Response<BondStatusResponse>, tonic::Status> {
        self.upstream.lock().await.bond_status(request).await
    }

    async fn exploratory_deploy(
        &self,
        request: tonic::Request<ExploratoryDeployQuery>,
    ) -> Result<tonic::Response<ExploratoryDeployResponse>, tonic::Status> {
        self.upstream.lock().await.exploratory_deploy(request).await
    }

    async fn get_blocks_by_heights(
        &self,
        request: tonic::Request<BlocksQueryByHeight>,
    ) -> Result<tonic::Response<Self::getBlocksByHeightsStream>, tonic::Status> {
        let response = self
            .upstream
            .lock()
            .await
            .get_blocks_by_heights(request)
            .await?;
        let stream = proxy_server_stream(response.into_inner()).await?;
        Ok(tonic::Response::new(stream))
    }

    async fn get_event_by_hash(
        &self,
        request: tonic::Request<ReportQuery>,
    ) -> Result<tonic::Response<EventInfoResponse>, tonic::Status> {
        self.upstream.lock().await.get_event_by_hash(request).await
    }

    async fn status(
        &self,
        request: tonic::Request<()>,
    ) -> Result<tonic::Response<StatusResponse>, tonic::Status> {
        self.upstream.lock().await.status(request).await
    }
}
