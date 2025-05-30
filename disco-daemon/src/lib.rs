pub mod config;
pub mod controller;
pub mod grpc;
pub mod network;
pub mod node;
pub mod raft_types;
pub mod settings;
pub mod store;

pub mod protobuf {
  tonic::include_proto!("disco");
}

mod pb_impl;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = protobuf::SetRequest,
        R = protobuf::Response,
        LeaderId = protobuf::LeaderId,
        Vote = protobuf::Vote,
        Entry = protobuf::Entry,
        Node = protobuf::Node,
        SnapshotData = Vec<u8>,
);

pub type NodeId = u64;
pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;

#[cfg(test)]
mod test;
