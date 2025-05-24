export async function init() {
  console.log("(init) Initializing provider...");

  let aws = await AwsProvider.init({
    name: "heavyobjects",
    region: "us-west-2",
    // profile: "default",
  });

  return new Cluster({
    name: "heavyobjects",
    provider: aws,
  });
}

export async function leader(cluster, node) {
  console.log(`(leader) ${cluster} - ${node}`);
}
