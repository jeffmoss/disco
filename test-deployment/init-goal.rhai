// Description: A simple deployment script that sets up a Disco cluster and deploys a web app

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
