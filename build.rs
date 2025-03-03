fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/*");
    let config = prost_build::Config::new();
    let proto_files = [
        "proto/internal_service.proto",
        "proto/management_service.proto",
        "proto/api_service.proto",
    ];
    tonic_build::configure()
        .type_attribute("raftd.Node", "#[derive(Eq, serde::Serialize, serde::Deserialize)]")
        .type_attribute(
            "raftd.SetRequest",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "raftd.Response",
            "#[derive(Eq, serde::Serialize, serde::Deserialize)]",
        )
        .compile_protos_with_config(config, &proto_files, &["proto"])?;
    Ok(())
}
