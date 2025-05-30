import { Provider } from "@disco/provider";
import { FileStorage } from "@disco/storage";
import { Cluster } from "@disco/cluster";

class AwsCluster extends Cluster {
  constructor(config) {
    console.log("(AwsCluster) Initializing cluster...");

    super(config);

    this.provider = new Provider.AWS({
      name: "heavyobjects",
      region: "us-west-2",
      // profile: "default",
    });

    this.storage = new FileStorage.S3({
      bucket: "heavyobjects-storage",
      provider: this.provider,
    });
  }

  async leader(node) {
    console.log(`(leader) ${cluster} - ${node}`);
  }
}

disco.start(
  new AwsCluster({
    name: "heavyobjects",
    region: ENV.AWS_REGION || "us-west-2",
    // profile: "default",
  })
);

disco.node.on("leader", async (node) => {
  console.log(`(leader) Node ${node} is now the leader.`);
  await disco.cluster.leader(node);
});
