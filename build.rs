fn main() -> Result<(), Box<dyn std::error::Error>> {
  println!("cargo:rerun-if-changed=src/*");
  let config = prost_build::Config::new();
  let proto_files = [
    "proto/raft.proto",
    "proto/app_types.proto",
    "proto/app.proto",
  ];

  // TODO: remove serde

  tonic_build::configure()
    .btree_map(["."])
    .type_attribute("raftd.Node", "#[derive(Eq)]")
    .type_attribute("raftd.SetRequest", "#[derive(Eq)]")
    .type_attribute("raftd.Response", "#[derive(Eq)]")
    .type_attribute("raftd.LeaderId", "#[derive(Eq)]")
    .type_attribute("raftd.Vote", "#[derive(Eq)]")
    .type_attribute("raftd.NodeIdSet", "#[derive(Eq)]")
    .type_attribute("raftd.Membership", "#[derive(Eq)]")
    .type_attribute("raftd.Entry", "#[derive(Eq)]")
    .compile_protos_with_config(config, &proto_files, &["proto"])?;
  Ok(())
}
