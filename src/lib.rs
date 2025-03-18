#![allow(clippy::uninlined_format_args)]

pub mod action;
pub mod client;
pub mod controller;
pub mod grpc;
pub mod network;
pub mod node;
pub mod raft_types;
pub mod settings;
pub mod store;

pub mod protobuf {
  tonic::include_proto!("raftd");
}

mod pb_impl;

#[cfg(test)]
mod test;

use crate::protobuf as pb;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = pb::SetRequest,
        R = pb::Response,
        LeaderId = pb::LeaderId,
        Vote = pb::Vote,
        Entry = pb::Entry,
        Node = pb::Node,
        SnapshotData = Vec<u8>,
);

pub type NodeId = u64;
pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;
