use crate::protobuf;

impl protobuf::SnapshotRequest {
  pub fn into_meta(self) -> Option<protobuf::SnapshotRequestMeta> {
    let p = self.payload?;
    match p {
      protobuf::snapshot_request::Payload::Meta(meta) => Some(meta),
      protobuf::snapshot_request::Payload::Chunk(_) => None,
    }
  }

  pub fn into_data_chunk(self) -> Option<Vec<u8>> {
    let p = self.payload?;
    match p {
      protobuf::snapshot_request::Payload::Meta(_) => None,
      protobuf::snapshot_request::Payload::Chunk(chunk) => Some(chunk),
    }
  }
}
