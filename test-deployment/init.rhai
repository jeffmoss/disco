let cluster = aws_cluster("test-deployment").region("us-west-2");

print(`Region: ${cluster.region}`);
