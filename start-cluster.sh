#!/bin/sh

set -o errexit

# Build type configuration
BUILD_TYPE=debug
NODE_COUNT=3
BASE_HOST=127.0.0.1
BASE_PORT=5051
EXECUTABLE="./target/$BUILD_TYPE/discod"

# Check if executable exists and build if needed
if [ ! -f "$EXECUTABLE" ]; then
    echo "Executable $EXECUTABLE not found. Building..."
    if [ "$BUILD_TYPE" = "debug" ]; then
        echo "Running cargo build for debug build..."
        cargo build
    elif [ "$BUILD_TYPE" = "release" ]; then
        echo "Running cargo build --release for release build..."
        cargo build --release
    fi
    
    # Check again if build succeeded
    if [ ! -f "$EXECUTABLE" ]; then
        echo "Error: Failed to build $EXECUTABLE"
        exit 1
    fi
fi

# Function to handle Ctrl+C (SIGINT) gracefully
cleanup() {
    echo -e "\nCtrl+C detected. Cleaning up..."
    echo "Killing all nodes..."
    pkill -f "discod"
    echo "All servers have been terminated."
    exit 0
}

# Set up trap for Ctrl+C
trap cleanup INT

rpc() {
    local port=$1
    local method=$2
    local body="$3"
    local isApiService="$4"
    cmd="grpcurl -plaintext -proto ./disco-daemon/proto/app.proto -d $body -import-path ./disco-daemon/proto localhost:$port disco.AppService/$method"

    echo '---'" rpc($BASE_HOST:$port/$method, $body)"

    {
	time $cmd
    } | {
        if type jq > /dev/null 2>&1; then
            jq 'if has("data") then .data |= fromjson else . end'
        else
            cat
        fi
    }

    echo
    echo
}

export RUST_LOG=trace
export RUST_BACKTRACE=full

echo "Killing all running discod instances..."

# Kill all running instances of discod
pkill -f "discod" || true

echo "Starting $NODE_COUNT uninitialized discod servers..."

# Start the servers in a loop
i=1
while [ $i -le $NODE_COUNT ]; do
    port=$((BASE_PORT + i - 1))
    $EXECUTABLE --id $i --addr $BASE_HOST:$port > n$i.log 2>&1 &
    echo "Server $i started at http://$BASE_HOST:$port"
    i=$((i + 1))
done

echo "\nAll servers are now running."
echo "Server logs are being written to n*.log files"

# Sleep 1 second before initializing the cluster
echo "\nWaiting 1 second before initializing the cluster..."
sleep 1

# Build the JSON array of nodes for the Init RPC call
nodes_json="{"
nodes_json="${nodes_json}\"nodes\":["

i=1
while [ $i -le $NODE_COUNT ]; do
    port=$((BASE_PORT + i - 1))
    
    # Add comma separator for all but the first node
    if [ $i -gt 1 ]; then
        nodes_json="${nodes_json},"
    fi
    
    nodes_json="${nodes_json}{\"node_id\":\"$i\",\"rpc_addr\":\"$BASE_HOST:$port\"}"
    i=$((i + 1))
done

nodes_json="${nodes_json}]}"

# Call Init RPC to initialize the cluster
rpc $BASE_PORT "Init" "$nodes_json"

echo "\nPress Ctrl+C to stop all servers and exit..."

while true; do
    sleep 1
done
