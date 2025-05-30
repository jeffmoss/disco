export async function init() {
  console.log("(init) Initializing provider...");

  let aws = await AwsProvider.init({
    name: "heavyobjects",
    region: "us-west-2",
    // profile: "default",
  });

  let role = await aws.role({
    name: "heavyobjects",
    // Default policy for now
  });

  // Initializes the bucket if it doesn't exist
  let storage = await aws.storage({
    bucket: "heavyobjects-storage",
    role,
  });

  return new Cluster({
    name: "heavyobjects",
    provider: aws,
    role,
    storage,
  });
}

export async function bootstrap(cluster) {
  console.log(`(bootstrap) ${cluster}`);
  // await delay(100).then((elapsed) => {
  //   console.log(`(bootstrap) Delay done! (elapsed: ${elapsed})`);
  // });

  if (await cluster.healthy()) {
    console.log(`(bootstrap) Cluster is ready, exiting...`);

    return;
  }

  let yes = await ask("Do you want to bootstrap the cluster?");
  if (!yes) {
    console.log(`(bootstrap) User declined to bootstrap, exiting...`);
    return;
  }

  console.log(`(bootstrap) Bootstrapping cluster...`);

  await cluster.set_key_pair({
    private: "./id_ed25519",
    public: "./id_ed25519.pub",
  });

  console.log("(bootstrap) Starting instance...");

  await cluster.start_instance({
    image: "ami-0e8c824f386e1de06", // Ubuntu 22.04 LTS
    instance_type: "t4g.micro",
  });

  console.log("(bootstrap) Attaching IP...");

  await cluster.attach_ip();

  await cluster.ssh_install();

  await cluster.scale(3);

  return `Complete cluster`;
}
