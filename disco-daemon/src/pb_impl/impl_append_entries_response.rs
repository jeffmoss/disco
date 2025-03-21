use crate::protobuf;
use crate::raft_types::AppendEntriesResponse;

impl From<protobuf::AppendEntriesResponse> for AppendEntriesResponse {
  fn from(r: protobuf::AppendEntriesResponse) -> Self {
    if let Some(higher) = r.rejected_by {
      return AppendEntriesResponse::HigherVote(higher);
    }

    if r.conflict {
      return AppendEntriesResponse::Conflict;
    }

    if let Some(log_id) = r.last_log_id {
      AppendEntriesResponse::PartialSuccess(Some(log_id.into()))
    } else {
      AppendEntriesResponse::Success
    }
  }
}

impl From<AppendEntriesResponse> for protobuf::AppendEntriesResponse {
  fn from(r: AppendEntriesResponse) -> Self {
    match r {
      AppendEntriesResponse::Success => protobuf::AppendEntriesResponse {
        rejected_by: None,
        conflict: false,
        last_log_id: None,
      },
      AppendEntriesResponse::PartialSuccess(p) => protobuf::AppendEntriesResponse {
        rejected_by: None,
        conflict: false,
        last_log_id: p.map(|log_id| log_id.into()),
      },
      AppendEntriesResponse::Conflict => protobuf::AppendEntriesResponse {
        rejected_by: None,
        conflict: true,
        last_log_id: None,
      },
      AppendEntriesResponse::HigherVote(v) => protobuf::AppendEntriesResponse {
        rejected_by: Some(v),
        conflict: false,
        last_log_id: None,
      },
    }
  }
}
