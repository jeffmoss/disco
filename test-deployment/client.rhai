fn bootstrap() {
  let cluster = aws_cluster("heavyobjects", "us-west-2");

  if (cluster.healthy()) {
    print("Cluster is ready, exiting...");
    return;
  }

  // Check for remnants of a previous cluster?
  // Check for running, tagged instances?

  let yes = ask("Do you want to bootstrap the cluster?");

  if (yes) {
    // Create a default key pair for the cluster.
    cluster.set_key_pair("./id_ed25519", "./id_ed25519.pub");

    let primary_node = cluster.start_instance("ami-0e8c824f386e1de06", "t4g.micro");

    // Bootstrap is only ever called if the primary_ip doesn't exist. Once
    // it is attached to a node it can be tranferred to another node upon
    // new leader election.
    cluster.attach_ip(primary_node, cluster.primary_ip());

    // Connect and upload the daemon and start it.
    cluster.ssh_install();

    // Scale the cluster to 3 nodes.
    cluster.scale(3);
  } else {
    print("Exiting...");
  }
}
