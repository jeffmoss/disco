use crate::protobuf;
use crate::raft_types::ClientWriteResponse;

impl From<protobuf::ClientWriteResponse> for ClientWriteResponse {
  fn from(r: protobuf::ClientWriteResponse) -> Self {
    ClientWriteResponse {
      log_id: r.log_id.unwrap().into(),
      data: r.data.unwrap(),
      membership: r.membership.map(|mem| mem.into()),
    }
  }
}

impl From<ClientWriteResponse> for protobuf::ClientWriteResponse {
  fn from(r: ClientWriteResponse) -> Self {
    protobuf::ClientWriteResponse {
      log_id: Some(r.log_id.into()),
      data: Some(r.data),
      membership: r.membership.map(|mem| mem.into()),
    }
  }
}
