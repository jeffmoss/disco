fn bootstrap() {
  let cluster = aws_cluster("heavyobjects", "us-west-2");

  if (cluster.healthy()) {
    print("Cluster is ready, exiting...");
    return;
  }

  // Check for remnants of a previous cluster?
  // Check for running, tagged instances?

  ask("Do you want to bootstrap the cluster? (y/n)", |answer| {
    if (answer == "y") {
      // Create a default key pair for the cluster.
      cluster.key_pair("./id_ed25519.pub");

      let primary_node = cluster.node("ami-0e8c824f386e1de06");

      // Bootstrap is only ever called if the primary_ip doesn't exist. Once
      // it is attached to a node it can be tranferred to another node upon
      // new leader election.
      primary_node.attach_ip(cluster.primary_ip());

      // Connect and upload the daemon and start it.
      primary_node.ssh_install();

      // Scale the cluster to 3 nodes.
      cluster.scale(3);
    } else {
      print("Exiting...");
    }
  )
}
