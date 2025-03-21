use std::fmt;

use openraft::vote::RaftVote;

use crate::protobuf;
use crate::raft_types::LeaderId;
use crate::TypeConfig;

impl RaftVote<TypeConfig> for protobuf::Vote {
  fn from_leader_id(leader_id: LeaderId, committed: bool) -> Self {
    protobuf::Vote {
      leader_id: Some(leader_id),
      committed,
    }
  }

  fn leader_id(&self) -> Option<&LeaderId> {
    self.leader_id.as_ref()
  }

  fn is_committed(&self) -> bool {
    self.committed
  }
}

impl fmt::Display for protobuf::Vote {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "<{}:{}>",
      self.leader_id.as_ref().unwrap_or(&Default::default()),
      if self.is_committed() { "Q" } else { "-" }
    )
  }
}
