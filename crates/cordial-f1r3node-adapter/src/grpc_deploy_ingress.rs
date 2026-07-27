use std::sync::Arc;

use either::Either;
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
    deploy_response, deploy_service_server::DeployService,
};
use models::casper::{
    BlockQuery, BlocksQuery, BlocksQueryByHeight, BondStatusQuery, ContinuationAtNameQuery,
    DataAtNameByBlockQuery, DeployDataProto, ExploratoryDeployQuery, FindDeployQuery,
    IsFinalizedQuery, LastFinalizedBlockQuery, MachineVerifyQuery, PrivateNamePreviewQuery,
    ReportQuery, VisualizeDagQuery,
};
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;

use crate::block_translation::SignedDeployData;
use crate::casper_adapter::{CasperError, DeployError, DeployId};
use crate::live_deploy_ingress::{GrpcDeployIngressError, LiveDeployIngress, ObservedDeploy};

/// AdmitFn is the admission callback: given a decoded signed deploy,
/// run it through the adapter's `deploy()` admission logic.
pub type AdmitFn = Arc<
    dyn Fn(SignedDeployData) -> Result<Either<DeployError, DeployId>, CasperError> + Send + Sync,
>;

#[derive(Debug)]
pub enum GrpcIngressError {
    Admission(GrpcDeployIngressError),
}

impl std::fmt::Display for GrpcIngressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admission(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for GrpcIngressError {}

pub struct GrpcDeployIngressHandler {
    ingress: Arc<Mutex<LiveDeployIngress>>,
    admit_fn: AdmitFn,
}

impl GrpcDeployIngressHandler {
    pub fn new(ingress: LiveDeployIngress, admit_fn: AdmitFn) -> Self {
        Self {
            ingress: Arc::new(Mutex::new(ingress)),
            admit_fn,
        }
    }

    pub fn from_arc(ingress: Arc<Mutex<LiveDeployIngress>>, admit_fn: AdmitFn) -> Self {
        Self { ingress, admit_fn }
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

    pub fn ingress(&self) -> &Arc<Mutex<LiveDeployIngress>> {
        &self.ingress
    }

    pub async fn handle_do_deploy_inner(
        &self,
        proto: DeployDataProto,
    ) -> Result<(ObservedDeploy, Either<DeployError, DeployId>), GrpcDeployIngressError> {
        let deploy = crate::live_deploy_ingress::grpc_proto_to_signed_deploy(&proto);
        let mut ingress = self.ingress.lock().await;
        let observed = ingress.observe_grpc_deploy(&deploy);
        let admission = (self.admit_fn)(deploy)?;
        Ok((observed, admission))
    }
}

#[tonic::async_trait]
impl DeployService for GrpcDeployIngressHandler {
    type visualizeDagStream = ReceiverStream<Result<VisualizeBlocksResponse, tonic::Status>>;
    type showMainChainStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;
    type getBlocksStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;
    type getBlocksByHeightsStream = ReceiverStream<Result<BlockInfoResponse, tonic::Status>>;

    async fn do_deploy(
        &self,
        request: tonic::Request<DeployDataProto>,
    ) -> Result<tonic::Response<DeployResponse>, tonic::Status> {
        let proto = request.into_inner();
        match self.handle_do_deploy_inner(proto).await {
            Ok((_observed, admission)) => {
                let message = match admission {
                    Either::Right(id) => format!("Success\nDeployId is: {}", hex::encode(&id)),
                    Either::Left(err) => {
                        return Err(tonic::Status::invalid_argument(format!("{err:?}")));
                    }
                };
                Ok(tonic::Response::new(DeployResponse {
                    message: Some(deploy_response::Message::Result(message)),
                }))
            }
            Err(err) => Err(tonic::Status::internal(format!("{err}"))),
        }
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

    #[cfg(f1r3node_has_listen_for_data_at_name)]
    async fn listen_for_data_at_name(
        &self,
        _request: tonic::Request<DataAtNameQuery>,
    ) -> Result<tonic::Response<ListeningNameDataResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("listenForDataAtName"))
    }

    #[cfg(f1r3node_has_deploy_finalization_status)]
    async fn deploy_finalization_status(
        &self,
        _request: tonic::Request<DeployFinalizationStatusQuery>,
    ) -> Result<tonic::Response<DeployFinalizationStatusResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("deployFinalizationStatus"))
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
        let status = models::casper::Status {
            address: "cordial-grpc-ingress".to_string(),
            network_id: "cordial-dev".to_string(),
            shard_id: "root".to_string(),
            peers: 0,
            nodes: 1,
            min_phlo_price: 1,
            ..Default::default()
        };
        Ok(tonic::Response::new(StatusResponse {
            message: Some(models::casper::v1::status_response::Message::Status(status)),
        }))
    }
}
