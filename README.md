# Disco

Disco is an opinionated, lightweight, distributed cloud orchestrator written in Rust. It presently uses less than 10mb of RAM which is ideal for situations where you want to maximize small cloud instances for a distributed cluster. It is designed to perform the same function as tools like Kubernetes, Terraform and Docker, making these tools optional for smaller simpler deployments.

The disco daemon employs the raft protocol for fault tolerance, ensuring that a single cluster controller is operational on one of the control plane nodes. Presently, the control plane consists of every node in the cluster. Cluster synchronization is all performed over gRPC channels. Each node is self-replicating and will start and stop other compute instances as instructed.

Cluster configuration and customization is scripted using ECMAScript. This is a departure from other systems that make heavy use of configuration files. Each node in the cluster (and the client) run a single asynchronous thread to handle all scripted operations such as health checks and deployments, with bindings for various higher and lower level events.

## Cluster setup

The easiest way to configure a cluster is to define a `cluster.js` and `client.js` file in your project directory. See the `test-deployment` for an example.

#### Example Script

`client.js`

```js
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

export async function bootstrap(cluster) {
  console.log(`(bootstrap) ${cluster}`);

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
```

`cluster.js`

```js
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
```

## Building & Installing

In addition to normal `cargo build` commands you can build static releases using Cross:

```bash
# Build local binaries
cargo build

# Cross-compile for aarch64
cross build --target aarch64-unknown-linux-musl --release

# Cross-compile for x86_64
cross build --target x86_64-unknown-linux-musl --release
```

You need a `disco` and `discod` executable built for any architecture you wish to run on.

When developing locally if you have [direnv](https://direnv.net/) installed you will automatically have the debug build in your path and can run `disco bootstrap` from the `test-deployment` directory.

During the `disco bootstrap`, symlinks in your `test-deployment` directory that reference `disco` and `discod` will be hydrated and installed onto the remote servers, so if running on an x86_64 host be sure to modify these symlinks to point to the proper target.
