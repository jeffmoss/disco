# Raftd

raftd is a lightweight container orchestrator compiled with rust. It uses less than 10mb of RAM, allowing you to utilize/maximize small cloud instances for a distributed cluster.

You can use a gRPC client (like `grpcurl`) to interact with the cluster. An sample 3-node cluster can be started with the `start-cluster.sh` script.

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
