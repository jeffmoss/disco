# Disco

Disco is an opinionated, lightweight, distributed cloud orchestrator written in Rust. It presently uses less than 10mb of RAM which is ideal for situations where you want to maximize small cloud instances for a distributed cluster. It is designed to perform the same function as tools like Kubernetes, Terraform and Docker, making these tools optional for smaller simpler deployments.

The disco daemon employs the raft protocol for fault tolerance, ensuring that a single cluster controller is operational on one of the control plane nodes. Presently, the control plane consists of every node in the cluster.

Cluster configuration is scripted using Rhai, an embedded scripting language for Rust. This is a departure from other systems that make heavy use of configuration files.

## Cluster setup

The easiest way to configure a cluster is to define a `cluster.dco` file in your project directory.

#### Example

```
// This is a Rhai script. Learn more about it at https://rhai.rs/book/

let provider = aws("us-west-2") // For now, everything has a default region
let repository = github("jeffmoss/disco")

let key_pair = provider.import_key_pair("disco-key")
  .public_key(local_file("./id_ed25519.pub"))

let cluster = provider.cluster("disco-primary")
  .image("ami-06db875b10d8a3ef8")
  .public_key(key_pair)
  .user("ubuntu") // The image default user
  .size(3, 5) // min: 3, max: 5
  .configure(
    // Install Node.js (once)
    local_file("./install_node.sh")
  )

// A standard set of configuration options can go in a function like this
fn configure_app(deployment, environment) {
  deployment
    .git(repository, "master")
    .ports(80, 443)
    .size(3, 12) // min: 3 (one on each node), max: 12 (4 on each node)
    .build_command("./build.js")
    .start_command("npx http-server -a 0.0.0.0 -o / dist")
    .environment("NODE_ENV", environment)
}

// Flexible deployment that simply clones the given git repo, builds and starts the HTTP service
let production = configure_app(cluster.deployment("web-app"), "production")
  .log_drain(provider.s3_log_bucket_drain("disco-web-app-logs"))

// With no log_drain defined in the testing environment, clients can stream logs
let testing = configure_app(cluster.deployment("web-app-testing"), "testing")

// CD pipeline to the testing environment using github actions
repository.branch("master").on("commit", |hash| testing.deploy(hash) )

// This script can access Disco's key-value store to trigger a production deployment manually.
disco.key("deployed-commit").on("change", |hash| production.deploy(hash) )

// Coming soon:
//  * containerized deployment with container registries
//  * TLS offloading
//  * autoscaling and metrics
//  * monitoring and alerting
//  * rollout strategies

// Finally, set up an ElasticIP to route traffic to the deployed application
let production_ingress = provider.elastic_ingress(provider.domain("disco.heavyobjects.com"))
  .ports(80, 443)
  .forward_to(production)

let testing_ingress = provider.elastic_ingress(provider.domain("disco-testing.heavyobjects.com"))
  .ports(80, 443)
  .forward_to(testing)
```

## Building

Currently supported build options:

```bash
# The simplest way to get up and running
cargo build --release

# Build a static binary
cargo build --release --features static

# Cross-compile a dynamically linked executable for AArch64 / glibc (ex. Ubuntu arm64 cloud hosts)
cross build --target aarch64-unknown-linux-gnu --release

# Cross-compile a statically linked executable for AArch64 / glibc
cross build --target aarch64-unknown-linux-gnu --release --features static
```

If you're compiling statically then you'll likely encounter an error `could not find the required libclang.a static library`. You'll need a custom build of LLVM in this case. You can run the following to install to `/usr/local`:

```bash
git clone --single-branch --depth=1 https://github.com/llvm/llvm-project.git
cd llvm-project
cmake -S llvm -B build -G Ninja \
  -DLLVM_ENABLE_PROJECTS="clang;lld" \
  -DLIBCLANG_BUILD_STATIC=ON \
  -DCMAKE_BUILD_TYPE=Release
ninja -C build install
```

You need a `disco` and `discod` executable built for any architecture you wish to run on.
