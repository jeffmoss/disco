// Description: A simple deployment script that sets up a Raft cluster and deploys a web app

// This is a Rhai script with the ability to fetch data from the cloud needed to configure the
// script, such as secret keys.

// Basic cluster initialization to set up 3 EC2 instances and start the raftd service
// Each node will have the given key

let provider = aws("us-west-2") // For now, everything has a default region
let repository = github("jeffmoss/raftd")

let key_pair = provider.import_key_pair("raftd-key")
  .public_key(
    // installed to each VM's user account by cloud-init
    local_file("./id_ed25519.pub")
  )

let cluster = provider.cluster("raftd-primary")
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
let production = configure_app(cluster.deployment("web-app"))
  .environment("NODE_ENV", "production")
  .log_drain(provider.s3_log_drain)

// With no log_drain defined in the testing environment, clients can stream logs
let testing = configure_app(cluster.deployment("web-app-testing"))
  .environment("NODE_ENV", "testing")

// CD pipeline to the testing environment using github actions
repository.branch("master").on("commit", |hash| testing.deploy(hash) )
// This script can access a raft key-value and that can also be used as a watch, allowing the client
// to trigger a production deployment manually.
raft.key("deployed-commit").on("change", |hash| production.deploy(hash) )

// TBD: containerized deployment with container registries
// TBD: TLS offloading
// TBD: autoscaling and metrics
// TBD: monitoring and alerting
// TBD: rollout strategies

// Finally, set up an ElasticIP to route traffic to the deployed application
let production_ingress = provider.elastic_ingress(provider.domain("raftd.heavyobjects.com"))
  .ports(80, 443)
  .forward_to(production)

let testing_ingress = provider.elastic_ingress(provider.domain("raftd-testing.heavyobjects.com"))
  .ports(80, 443)
  .forward_to(testing)
