use crate::protobuf;
use crate::raft_types::LogId;

impl From<LogId> for protobuf::LogId {
  fn from(log_id: LogId) -> Self {
    protobuf::LogId {
      term: *log_id.committed_leader_id(),
      index: log_id.index(),
    }
  }
}

impl From<protobuf::LogId> for LogId {
  fn from(proto_log_id: protobuf::LogId) -> Self {
    LogId::new(proto_log_id.term, proto_log_id.index)
  }
}
