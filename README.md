# Raftd

raftd is a lightweight container orchestrator compiled with rust. It uses only 20mb of RAM, allowing you to use very small EC2 instances for a distributed cluster.

The key components are the openraft code, a RocksDB node storage database, and a gRPC communication layer.

You can use a gRPC client (like `grpcurl`) to interact with the cluster.

## Example

```bash
# Start node 1
raftd --id 1 --addr 127.0.0.1:10101

# Start node 2
raftd --id 2 --addr 127.0.0.1:10102

# Start node 3
raftd --id 3 --addr 127.0.0.1:10103


grpcurl -plaintext -proto ./proto/management_service.proto \
  -d '\
    {"nodes":[ \
      {"node_id":"1","rpc_addr":"127.0.0.1:10101"}, \
      {"node_id":"2","rpc_addr":"127.0.0.1:10102"}, \
      {"node_id":"3","rpc_addr":"127.0.0.1:10103"} \
    ]}' \
  -import-path ./proto \
  localhost:10101 raftd.ManagementService/Init

grpcurl -plaintext -proto ./proto/api_service.proto \
  -d '{"key":"foo","value":"bar"}' \
  -import-path ./proto \
  localhost:10101 raftd.ApiService/Set

grpcurl -plaintext -proto ./proto/api_service.proto \
  -d '{"key":"foo"}' \
  -import-path ./proto \
  localhost:10102 raftd.ApiService/Get
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
