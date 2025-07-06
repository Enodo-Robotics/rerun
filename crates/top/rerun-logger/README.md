# Rerun Logger

A standalone command-line application that extracts the data buffering and periodic file saving functionality from the Rerun viewer.

## Features

- **Data Buffering**: Efficiently buffers incoming Rerun log messages in memory
- **Periodic Saving**: Automatically saves data to `.rrd` files based on configurable thresholds
- **gRPC Server**: Accepts data from Rerun SDKs via gRPC protocol
- **Memory Management**: Configurable memory limits and garbage collection
- **CLI Interface**: Easy-to-use command-line interface with comprehensive options

## Usage

### Basic Usage

```bash
# Start logger with default settings, saving to output.rrd
rerun-logger --output output.rrd

# Start with custom flush intervals and memory limits
rerun-logger --output data.rrd --flush-interval 100ms --max-memory 512MB

# Listen on custom gRPC port
rerun-logger --output logs.rrd --port 9877
```

### Environment Variables

You can configure the logger using environment variables:

- `RERUN_FLUSH_TICK_SECS`: Flush frequency in seconds (default: 0.008)
- `RERUN_FLUSH_NUM_BYTES`: Flush threshold in bytes (default: 1048576)
- `RERUN_FLUSH_NUM_ROWS`: Flush threshold in number of rows (default: unlimited)
- `RERUN_LOGGER_MAX_MEMORY`: Maximum memory usage before forced flush

### CLI Options

```
USAGE:
    rerun-logger [OPTIONS] --output <OUTPUT>

OPTIONS:
    -o, --output <OUTPUT>              Output .rrd file path
    -p, --port <PORT>                  gRPC server port [default: 9876]
        --flush-interval <DURATION>    Flush interval (e.g., 50ms, 1s) [default: 8ms]
        --flush-bytes <BYTES>          Flush when buffer reaches this size [default: 1MB]
        --flush-rows <ROWS>            Flush when buffer reaches this many rows
        --max-memory <BYTES>           Maximum memory usage before forced flush
        --compression <LEVEL>          Compression level for .rrd files [default: fast]
    -v, --verbose                      Verbose logging
    -q, --quiet                        Quiet mode (errors only)
    -h, --help                         Print help
    -V, --version                      Print version
```

## Examples

### Log Data from Python SDK

1. Start the logger:
```bash
rerun-logger --output my_data.rrd --port 9876
```

2. Use Python SDK to send data:
```python
import rerun as rr

rr.init("my_app")
rr.connect("127.0.0.1:9876")

# Your logging code here
rr.log("points", rr.Points3D([[1, 2, 3], [4, 5, 6]]))
```

### High-Frequency Data Logging

For high-frequency data, adjust the flush parameters:

```bash
rerun-logger --output high_freq.rrd --flush-interval 1s --flush-bytes 10MB
```

## Building

```bash
cargo build --release
```

## Testing

```bash
cargo test
```