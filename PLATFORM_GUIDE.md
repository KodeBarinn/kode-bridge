# Cross-Platform Usage Guide

This guide explains how to use the kode-bridge library and its examples/benchmarks on different platforms.

## Examples

### Basic Request Example
```bash
cargo run --example request
```

### Large Request Example  
```bash
cargo run --example request_large
```

## Benchmarks

### Running Benchmarks
```bash
cargo bench
```

## Environment Variables

The library automatically detects your platform and uses appropriate environment variables:

### Unix (Linux, macOS, etc.)
- **Variable**: `CUSTOM_SOCK`
- **Default**: `/tmp/custom.sock`
- **Example**: 
  ```bash
  CUSTOM_SOCK=/tmp/my_socket cargo run --example request
  ```

### Windows
- **Variable**: `CUSTOM_PIPE` 
- **Default**: `\\.\pipe\mihomo`
- **Example**:
  ```cmd
  set CUSTOM_PIPE=\\.\pipe\my_pipe
  cargo run --example request
  ```

## .env File Support

You can also create a `.env` file in the project root:

### Unix .env example:
```env
CUSTOM_SOCK=/path/to/your/socket
```

### Windows .env example:
```env
CUSTOM_PIPE=\\.\pipe\your_pipe
```

## Platform-Specific Notes

### Unix
- Uses Unix domain sockets
- Better performance with connection reuse in benchmarks
- Default path: `/tmp/custom.sock`

### Windows
- Uses named pipes
- Benchmarks use reduced sample size to avoid connection issues
- Creates new client per benchmark iteration for stability
- Default path: `\\.\pipe\mihomo`

## Migration from Platform-Specific Code

If you were previously using platform-specific imports:

### Before:
```rust
#[cfg(unix)]
use kode_bridge::ipc_cilent::unix::UnixIpcHttpClient as IpcHttpClient;
#[cfg(windows)] 
use kode_bridge::ipc_cilent::windows::WindowsIpcHttpClient as IpcHttpClient;
```

### Now:
```rust
use kode_bridge::IpcHttpClient;
```

The API remains exactly the same - only the import has been simplified!
