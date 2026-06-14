use std::collections::{HashMap, HashSet};

use cordial_miners_core::block::Block;
use cordial_miners_core::execution::{BlockState, Bond, CordialBlockPayload};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use models::casper::v1::{
    block_info_response, block_response, deploy_service_client::DeployServiceClient,
    last_finalized_block_response,
};
use models::casper::{
    BlockQuery, BlocksQuery, BlocksQueryByHeight, LastFinalizedBlockQuery, LightBlockInfo,
};
use models::rust::string_ops::StringOps;
use tonic::transport::{Channel, Endpoint};

use crate::block_translation::{
    BlockMessage, Bond as AdapterBond, F1r3flyState, Header, Justification,
};
use crate::grpc_ingest::BlocklaceAdapter;
use crate::live_ingress::{LiveIngress, LiveIngressError, MirrorDisposition};

#[derive(Debug)]
pub enum LiveGrpcError {
    Transport(tonic::transport::Error),
    Status(tonic::Status),
    MissingPayload(&'static str),
    DecodeHex { field: &'static str, value: String },
    InvalidHashLength { field: &'static str, got: usize },
    NumericOverflow(&'static str),
    Ingress(LiveIngressError),
}

impl std::fmt::Display for LiveGrpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(err) => write!(f, "failed to connect to live gRPC endpoint: {err}"),
            Self::Status(err) => write!(f, "live gRPC request failed: {err}"),
            Self::MissingPayload(kind) => write!(f, "live gRPC response missing payload: {kind}"),
            Self::DecodeHex { field, value } => {
                write!(f, "failed to decode hex field {field}: {value}")
            }
            Self::InvalidHashLength { field, got } => {
                write!(f, "field {field} must decode to 32 bytes, got {got}")
            }
            Self::NumericOverflow(field) => write!(f, "numeric overflow while converting {field}"),
            Self::Ingress(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LiveGrpcError {}

pub struct LiveGrpcBlockClient {
    client: DeployServiceClient<Channel>,
}

impl LiveGrpcBlockClient {
    pub async fn connect(uri: impl Into<String>) -> Result<Self, LiveGrpcError> {
        let endpoint = Endpoint::from_shared(uri.into()).map_err(LiveGrpcError::Transport)?;
        let channel = endpoint.connect().await.map_err(LiveGrpcError::Transport)?;
        Ok(Self {
            client: DeployServiceClient::new(channel),
        })
    }

    pub async fn recent_light_blocks(
        &mut self,
        depth: i32,
    ) -> Result<Vec<LightBlockInfo>, LiveGrpcError> {
        let mut stream = self
            .client
            .get_blocks(BlocksQuery { depth })
            .await
            .map_err(LiveGrpcError::Status)?
            .into_inner();

        let mut blocks = Vec::new();
        while let Some(item) = stream.message().await.map_err(LiveGrpcError::Status)? {
            match item.message {
                Some(block_info_response::Message::BlockInfo(light)) => blocks.push(light),
                Some(block_info_response::Message::Error(status)) => {
                    return Err(LiveGrpcError::MissingPayload(Box::leak(
                        format!("get_blocks returned service error: {:?}", status).into_boxed_str(),
                    )));
                }
                None => return Err(LiveGrpcError::MissingPayload("get_blocks.message")),
            }
        }

        blocks.sort_by(|a, b| {
            a.block_number
                .cmp(&b.block_number)
                .then_with(|| a.block_hash.cmp(&b.block_hash))
        });
        Ok(blocks)
    }

    pub async fn light_blocks_by_heights(
        &mut self,
        start_block_number: i64,
        end_block_number: i64,
    ) -> Result<Vec<LightBlockInfo>, LiveGrpcError> {
        let mut stream = self
            .client
            .get_blocks_by_heights(BlocksQueryByHeight {
                start_block_number,
                end_block_number,
            })
            .await
            .map_err(LiveGrpcError::Status)?
            .into_inner();

        let mut blocks = Vec::new();
        while let Some(item) = stream.message().await.map_err(LiveGrpcError::Status)? {
            match item.message {
                Some(block_info_response::Message::BlockInfo(light)) => blocks.push(light),
                Some(block_info_response::Message::Error(status)) => {
                    return Err(LiveGrpcError::MissingPayload(Box::leak(
                        format!("get_blocks_by_heights returned service error: {:?}", status)
                            .into_boxed_str(),
                    )));
                }
                None => {
                    return Err(LiveGrpcError::MissingPayload(
                        "get_blocks_by_heights.message",
                    ));
                }
            }
        }

        blocks.sort_by(|a, b| {
            a.block_number
                .cmp(&b.block_number)
                .then_with(|| a.block_hash.cmp(&b.block_hash))
        });
        Ok(blocks)
    }

    pub async fn light_block_by_hash(
        &mut self,
        hash: impl Into<String>,
    ) -> Result<LightBlockInfo, LiveGrpcError> {
        let response = self
            .client
            .get_block(BlockQuery { hash: hash.into() })
            .await
            .map_err(LiveGrpcError::Status)?
            .into_inner();

        match response.message {
            Some(block_response::Message::BlockInfo(block_info)) => block_info
                .block_info
                .ok_or(LiveGrpcError::MissingPayload("get_block.block_info")),
            Some(block_response::Message::Error(status)) => {
                Err(LiveGrpcError::MissingPayload(Box::leak(
                    format!("get_block returned service error: {:?}", status).into_boxed_str(),
                )))
            }
            None => Err(LiveGrpcError::MissingPayload("get_block.message")),
        }
    }

    pub async fn last_finalized_block_hash(&mut self) -> Result<Option<Vec<u8>>, LiveGrpcError> {
        let response = self
            .client
            .last_finalized_block(LastFinalizedBlockQuery {})
            .await
            .map_err(LiveGrpcError::Status)?
            .into_inner();

        match response.message {
            Some(last_finalized_block_response::Message::BlockInfo(block_info)) => {
                let light = block_info.block_info.ok_or(LiveGrpcError::MissingPayload(
                    "last_finalized_block.block_info",
                ))?;
                Ok(Some(
                    decode_hex_32("block_hash", &light.block_hash)?.to_vec(),
                ))
            }
            Some(last_finalized_block_response::Message::Error(_)) => Ok(None),
            None => Err(LiveGrpcError::MissingPayload(
                "last_finalized_block.message",
            )),
        }
    }

    pub async fn ingest_recent_blocks<A>(
        &mut self,
        ingress: &mut LiveIngress<A>,
        depth: i32,
    ) -> Result<usize, LiveGrpcError>
    where
        A: BlocklaceAdapter<BlockIdentity>,
    {
        let blocks = self.recent_light_blocks(depth).await?;
        let mut applied = 0usize;

        for info in blocks {
            let block = trusted_block_from_light_block_info(&info)?;
            let update = ingress
                .ingest_trusted_block(block)
                .map_err(LiveGrpcError::Ingress)?;
            if update.disposition != MirrorDisposition::Duplicate {
                applied += 1;
            }
        }

        Ok(applied)
    }
}

pub fn trusted_block_from_light_block_info(info: &LightBlockInfo) -> Result<Block, LiveGrpcError> {
    trusted_block_from_light_block_info_with_options(info, true)
}

pub fn trusted_block_from_light_block_info_with_options(
    info: &LightBlockInfo,
    include_justifications: bool,
) -> Result<Block, LiveGrpcError> {
    let mut justification_creators: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    for justification in &info.justifications {
        let hash = decode_hex("latest_block_hash", &justification.latest_block_hash)?;
        let creator = decode_hex("validator", &justification.validator)?;
        justification_creators.insert(hash, creator);
    }

    let mut predecessors = HashSet::new();
    for hash in &info.parents_hash_list {
        let hash_bytes = decode_hex_32("parents_hash_list", hash)?;
        let creator = justification_creators
            .get(hash_bytes.as_slice())
            .cloned()
            .unwrap_or_default();
        predecessors.insert(BlockIdentity {
            content_hash: hash_bytes,
            creator: NodeId(creator),
            signature: vec![],
        });
    }

    if include_justifications {
        for justification in &info.justifications {
            predecessors.insert(BlockIdentity {
                content_hash: decode_hex_32("latest_block_hash", &justification.latest_block_hash)?,
                creator: NodeId(decode_hex("validator", &justification.validator)?),
                signature: vec![],
            });
        }
    }

    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: decode_hex("pre_state_hash", &info.pre_state_hash)?,
            post_state_hash: decode_hex("post_state_hash", &info.post_state_hash)?,
            bonds: info
                .bonds
                .iter()
                .map(|bond| {
                    Ok(Bond {
                        validator: NodeId(decode_hex("bond.validator", &bond.validator)?),
                        stake: u64::try_from(bond.stake)
                            .map_err(|_| LiveGrpcError::NumericOverflow("bond.stake"))?,
                    })
                })
                .collect::<Result<Vec<_>, LiveGrpcError>>()?,
            block_number: u64::try_from(info.block_number)
                .map_err(|_| LiveGrpcError::NumericOverflow("block_number"))?,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    };

    Ok(Block {
        identity: BlockIdentity {
            content_hash: decode_hex_32("block_hash", &info.block_hash)?,
            creator: NodeId(decode_hex("sender", &info.sender)?),
            signature: decode_hex("sig", &info.sig)?,
        },
        content: BlockContent {
            payload: payload.to_bytes(),
            predecessors,
        },
    })
}

pub fn light_block_info_to_block_message(
    info: &LightBlockInfo,
) -> Result<BlockMessage, LiveGrpcError> {
    Ok(BlockMessage {
        block_hash: decode_hex("block_hash", &info.block_hash)?,
        header: Header {
            parents_hash_list: info
                .parents_hash_list
                .iter()
                .map(|hash| decode_hex("parents_hash_list", hash))
                .collect::<Result<Vec<_>, _>>()?,
            timestamp: info.timestamp,
            version: info.version,
            extra_bytes: info.header_extra_bytes.to_vec(),
        },
        body: crate::block_translation::Body {
            state: F1r3flyState {
                pre_state_hash: decode_hex("pre_state_hash", &info.pre_state_hash)?,
                post_state_hash: decode_hex("post_state_hash", &info.post_state_hash)?,
                bonds: info
                    .bonds
                    .iter()
                    .map(|bond| {
                        Ok(AdapterBond {
                            validator: decode_hex("bond.validator", &bond.validator)?,
                            stake: bond.stake,
                        })
                    })
                    .collect::<Result<Vec<_>, LiveGrpcError>>()?,
                block_number: info.block_number,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: info.body_extra_bytes.to_vec(),
        },
        justifications: info
            .justifications
            .iter()
            .map(|justification| {
                Ok(Justification {
                    validator: decode_hex("validator", &justification.validator)?,
                    latest_block_hash: decode_hex(
                        "latest_block_hash",
                        &justification.latest_block_hash,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, LiveGrpcError>>()?,
        sender: decode_hex("sender", &info.sender)?,
        seq_num: i32::try_from(info.seq_num)
            .map_err(|_| LiveGrpcError::NumericOverflow("seq_num"))?,
        sig: decode_hex("sig", &info.sig)?,
        sig_algorithm: info.sig_algorithm.clone(),
        shard_id: info.shard_id.clone(),
        extra_bytes: info.extra_bytes.to_vec(),
    })
}

fn decode_hex(field: &'static str, value: &str) -> Result<Vec<u8>, LiveGrpcError> {
    StringOps::decode_hex(value.to_string()).ok_or_else(|| LiveGrpcError::DecodeHex {
        field,
        value: value.to_string(),
    })
}

fn decode_hex_32(field: &'static str, value: &str) -> Result<[u8; 32], LiveGrpcError> {
    let bytes = decode_hex(field, value)?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| LiveGrpcError::InvalidHashLength {
        field,
        got: bytes.len(),
    })
}
