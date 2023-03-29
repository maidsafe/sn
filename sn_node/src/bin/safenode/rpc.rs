use super::NodeCtrl;

use sn_node::node::NodeRef;

use color_eyre::eyre::{ErrReport, Result};
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{
    mpsc::{self, Sender},
    RwLock,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Code, Request, Response, Status};
use tracing::{debug, info, trace};

use safenode::safe_node_server::{SafeNode, SafeNodeServer};
use safenode::{
    NodeEvent, NodeEventsRequest, NodeInfoRequest, NodeInfoResponse, RestartRequest,
    RestartResponse, SectionMember, SectionMembersRequest, SectionMembersResponse, StopRequest,
    StopResponse, UpdateRequest, UpdateResponse,
};

// this would include code generated from .proto file
#[allow(unused_qualifications, clippy::unwrap_used)]
mod safenode {
    tonic::include_proto!("safenode");
}

// Defining a struct to hold information used by our gRPC service backend
struct SafeNodeRpcService {
    addr: SocketAddr,
    log_dir: String,
    node_ref: Arc<RwLock<NodeRef>>,
    ctrl_tx: Sender<NodeCtrl>,
}

// Implementing RPC interface for service defined in .proto
#[tonic::async_trait]
impl SafeNode for SafeNodeRpcService {
    type NodeEventsStream = ReceiverStream<Result<NodeEvent, Status>>;

    async fn node_info(
        &self,
        request: Request<NodeInfoRequest>,
    ) -> Result<Response<NodeInfoResponse>, Status> {
        trace!(
            "RPC request received at {}: {:?}",
            self.addr,
            request.get_ref()
        );
        let context = &self.node_ref.read().await.context;
        let resp = Response::new(NodeInfoResponse {
            node_name: context.name().0.to_vec(),
            is_elder: context.is_elder(),
            log_dir: self.log_dir.clone(),
            bin_version: env!("CARGO_PKG_VERSION").to_string(),
        });

        Ok(resp)
    }

    async fn section_members(
        &self,
        request: Request<SectionMembersRequest>,
    ) -> Result<Response<SectionMembersResponse>, Status> {
        trace!(
            "RPC request received at {}: {:?}",
            self.addr,
            request.get_ref()
        );
        let network_knowledge = self
            .node_ref
            .read()
            .await
            .context
            .network_knowledge()
            .clone();
        let section_members = network_knowledge
            .members()
            .into_iter()
            .map(|node_id| SectionMember {
                node_name: node_id.name().0.to_vec(),
                is_elder: network_knowledge.is_elder(&node_id.name()),
                addr: format!("{}", node_id.addr()),
            })
            .collect();

        let resp = Response::new(SectionMembersResponse { section_members });

        Ok(resp)
    }

    async fn node_events(
        &self,
        request: Request<NodeEventsRequest>,
    ) -> Result<Response<Self::NodeEventsStream>, Status> {
        trace!(
            "RPC request received at {}: {:?}",
            self.addr,
            request.get_ref()
        );

        let (client_tx, client_rx) = mpsc::channel(4);

        let mut events_rx = self.node_ref.read().await.events_channel.subscribe();
        let _handle = tokio::spawn(async move {
            while let Ok(event) = events_rx.recv().await {
                let event = NodeEvent {
                    event: format!("Event-{event}"),
                };

                if let Err(err) = client_tx.send(Ok(event)).await {
                    debug!(
                        "Dropping stream sender to RPC client due to failure in \
                        last attempt to notify an event: {err}"
                    );
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(client_rx)))
    }

    async fn stop(&self, request: Request<StopRequest>) -> Result<Response<StopResponse>, Status> {
        trace!(
            "RPC request received at {}: {:?}",
            self.addr,
            request.get_ref()
        );

        let cause = if let Some(addr) = request.remote_addr() {
            ErrReport::msg(format!(
                "Node has been stopped by an RPC request from {addr}."
            ))
        } else {
            ErrReport::msg("Node has been stopped by an RPC request from an unknown address.")
        };

        let delay = Duration::from_millis(request.get_ref().delay_millis);
        match self.ctrl_tx.send(NodeCtrl::Stop { delay, cause }).await {
            Ok(()) => Ok(Response::new(StopResponse {})),
            Err(err) => Err(Status::new(
                Code::Internal,
                format!("Failed to stop the node: {err}"),
            )),
        }
    }

    async fn restart(
        &self,
        request: Request<RestartRequest>,
    ) -> Result<Response<RestartResponse>, Status> {
        trace!(
            "RPC request received at {}: {:?}",
            self.addr,
            request.get_ref()
        );

        let delay = Duration::from_millis(request.get_ref().delay_millis);
        match self.ctrl_tx.send(NodeCtrl::Restart(delay)).await {
            Ok(()) => Ok(Response::new(RestartResponse {})),
            Err(err) => Err(Status::new(
                Code::Internal,
                format!("Failed to restart the node: {err}"),
            )),
        }
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        trace!(
            "RPC request received at {}: {:?}",
            self.addr,
            request.get_ref()
        );

        let delay = Duration::from_millis(request.get_ref().delay_millis);
        match self.ctrl_tx.send(NodeCtrl::Update(delay)).await {
            Ok(()) => Ok(Response::new(UpdateResponse {})),
            Err(err) => Err(Status::new(
                Code::Internal,
                format!("Failed to update the node: {err}"),
            )),
        }
    }
}

pub(super) fn start_rpc_service(
    addr: SocketAddr,
    log_dir: String,
    node_ref: Arc<RwLock<NodeRef>>,
    ctrl_tx: Sender<NodeCtrl>,
) {
    // creating a service
    let service = SafeNodeRpcService {
        addr,
        log_dir,
        node_ref,
        ctrl_tx,
    };
    info!("RPC Server listening on {addr}");
    println!("RPC Server listening on {addr}");

    let _handle = tokio::spawn(async move {
        // adding our service to our server.
        Server::builder()
            .add_service(SafeNodeServer::new(service))
            .serve(addr)
            .await
    });
}
