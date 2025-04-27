let provider = DISCO.aws("heavyobjects")
  .access_key_id("AKIAIOSFODNN7EXAMPLE");
  .secret_access_key("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
  .default_region("us-west-2")

let key_pair = provider.key_pair("heavyobjects-key")
  .public_key_file("./id_ed25519.pub")

let cluster = provider.cluster("heavyobjects-cluster")
  .instance_type("t4g.micro")
  .image("ami-0e8c824f386e1de06") // Ubuntu 24.04 LTS (Noble Numbat) - arm64 - us-west-2
  .key_pair(key_pair)
  .user("ubuntu")
  .size(3);

import "cluster" as cluster;
import "node" as node;

// Run at the start of each client or server process to set up the environment
fn init(disco) {
  disco.provider("aws")

  // Set up the cluster's behavior
  disco.cluster(#{
    // Set up the cluster's behavior
    start: start,
    stop: stop,
    scale: scale,
    deploy: deploy,
    undeploy: undeploy,
    configure: configure,
    status: status,
    logs: logs,
    metrics: metrics,
    events: events,
    teardown: teardown,
  })
}

fn start(disco, cluster) {
  // Start the cluster
  cluster.start()
}