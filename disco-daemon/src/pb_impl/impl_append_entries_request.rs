use crate::protobuf;

use crate::raft_types::AppendEntriesRequest;

impl From<protobuf::AppendEntriesRequest> for AppendEntriesRequest {
  fn from(proto_req: protobuf::AppendEntriesRequest) -> Self {
    AppendEntriesRequest {
      vote: proto_req.vote.unwrap(),
      prev_log_id: proto_req.prev_log_id.map(|log_id| log_id.into()),
      entries: proto_req.entries,
      leader_commit: proto_req.leader_commit.map(|log_id| log_id.into()),
    }
  }
}

impl From<AppendEntriesRequest> for protobuf::AppendEntriesRequest {
  fn from(value: AppendEntriesRequest) -> Self {
    protobuf::AppendEntriesRequest {
      vote: Some(value.vote),
      prev_log_id: value.prev_log_id.map(|log_id| log_id.into()),
      entries: value.entries,
      leader_commit: value.leader_commit.map(|log_id| log_id.into()),
    }
  }
}
