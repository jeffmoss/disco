use crate::protobuf;
use crate::raft_types::VoteRequest;

impl From<VoteRequest> for protobuf::VoteRequest {
  fn from(vote_req: VoteRequest) -> Self {
    protobuf::VoteRequest {
      vote: Some(vote_req.vote),
      last_log_id: vote_req.last_log_id.map(|log_id| log_id.into()),
    }
  }
}

impl From<protobuf::VoteRequest> for VoteRequest {
  fn from(proto_vote_req: protobuf::VoteRequest) -> Self {
    let vote = proto_vote_req.vote.unwrap();
    let last_log_id = proto_vote_req.last_log_id.map(|log_id| log_id.into());
    VoteRequest::new(vote, last_log_id)
  }
}
