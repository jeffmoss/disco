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
    .type_attribute("disco.Node", "#[derive(Eq)]")
    .type_attribute("disco.SetRequest", "#[derive(Eq)]")
    .type_attribute("disco.Response", "#[derive(Eq)]")
    .type_attribute("disco.LeaderId", "#[derive(Eq)]")
    .type_attribute("disco.Vote", "#[derive(Eq)]")
    .type_attribute("disco.NodeIdSet", "#[derive(Eq)]")
    .type_attribute("disco.Membership", "#[derive(Eq)]")
    .type_attribute("disco.Entry", "#[derive(Eq)]")
    .compile_protos_with_config(config, &proto_files, &["proto"])?;
  Ok(())
}
