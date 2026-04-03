# Soroban Resource Limits and WASM Size

This document outlines the resource constraints and WASM size considerations for the StreamPay Soroban smart contract.

## WASM Size Limits

Soroban enforces strict limits on contract WASM size to ensure efficient on-chain execution and storage:

- **Maximum contract size**: 256 KB (262,144 bytes) uncompressed WASM
- **Recommended target**: < 100 KB for optimal performance and future-proofing
- **Current contract size**: See CI build output or run `./scripts/check-wasm-size.sh`

### Why WASM Size Matters

1. **Network efficiency**: Smaller contracts reduce deployment costs and network bandwidth
2. **Storage costs**: Contract storage fees scale with size
3. **Upgrade headroom**: Keeping contracts small leaves room for future features
4. **Performance**: Smaller WASM modules load and instantiate faster

### Monitoring Strategy

The CI pipeline automatically reports WASM size on every build:
- Optimized release build size (with `opt-level = "z"`)
- Comparison against the 256 KB hard limit
- Warning threshold at 200 KB (78% of limit)

## Soroban Resource Limits

Soroban enforces per-invocation resource limits to ensure fair network usage and prevent abuse.

### CPU Instructions

- **Limit**: 100 million instructions per invocation
- **StreamPay impact**: Simple operations (create, start, stop) use < 1M instructions
- **Settlement operations**: Scale linearly with time elapsed but remain well under limits

### Memory

- **Limit**: 40 MB per invocation
- **StreamPay impact**: Minimal memory usage; `StreamInfo` struct is ~200 bytes
- **Storage**: Each stream is an independent persistent entry

### Ledger I/O

- **Read bytes**: 200 KB per invocation
- **Write bytes**: 100 KB per invocation
- **StreamPay impact**: Single stream operations read/write < 1 KB

### Ledger Entry Access

- **Read-only entries**: 40 per invocation
- **Read-write entries**: 25 per invocation
- **StreamPay impact**: Typically 1-2 entries per operation (stream + instance storage)

### Transaction Limits

- **Maximum transaction size**: 100 KB
- **Maximum operations per transaction**: 100
- **StreamPay impact**: All operations fit comfortably within limits

## Storage Model and TTL

StreamPay uses Soroban's persistent storage with automatic TTL management:

### TTL Configuration

```rust
const STREAM_TTL_THRESHOLD: u32 = 17_280;  // ~1 day (at ~5s/ledger)
const STREAM_TTL_EXTEND: u32 = 518_400;    // ~30 days
const INSTANCE_TTL_THRESHOLD: u32 = 17_280;
const INSTANCE_TTL_EXTEND: u32 = 518_400;
```

### Storage Costs

- **Persistent storage**: Rent-based model; TTL extensions cost XLM
- **Instance storage**: Contract-level metadata (next_id counter)
- **Per-stream cost**: ~200 bytes + TTL rent for 30 days

### Best Practices

1. **Archive settled streams**: Use `archive_stream()` to remove fully-settled streams and reclaim storage
2. **Monitor TTL**: Streams auto-extend on interaction; inactive streams may expire
3. **Batch operations**: Consider batching multiple stream operations in a single transaction

## Official Documentation

For the latest Soroban resource limits and fee structure:

- [Soroban Resource Limits](https://developers.stellar.org/docs/learn/smart-contract-internals/resource-limits-fees)
- [Soroban Fees](https://developers.stellar.org/docs/learn/smart-contract-internals/fees-and-metering)
- [Contract Storage](https://developers.stellar.org/docs/learn/smart-contract-internals/persisting-data)
- [State Archival](https://developers.stellar.org/docs/learn/smart-contract-internals/state-archival)

## Optimization Strategies

### Code Size Reduction

1. **Cargo profile optimization**: Use `opt-level = "z"` for size optimization
2. **Minimize dependencies**: Each dependency adds to WASM size
3. **Avoid generics**: Generic code can bloat WASM through monomorphization
4. **Use `#[inline(never)]`**: Prevent aggressive inlining of large functions

### Resource Efficiency

1. **Minimize storage reads**: Cache frequently accessed data
2. **Batch TTL extensions**: Extend multiple entries in one call when possible
3. **Use appropriate storage types**: Instance vs. Persistent vs. Temporary
4. **Optimize data structures**: Smaller structs = lower storage costs

## Regression Prevention

The CI pipeline includes automated checks:

1. **WASM size reporting**: Every build reports optimized size
2. **Threshold warnings**: Alert when approaching 200 KB (non-blocking)
3. **Test coverage**: Maintain 95%+ coverage to catch resource-heavy code paths
4. **Manual review**: Large size increases require explicit justification in PRs

## Current Contract Analysis

### StreamInfo Structure

```rust
pub struct StreamInfo {
    pub payer: Address,           // 32 bytes
    pub recipient: Address,       // 32 bytes
    pub rate_per_second: i128,    // 16 bytes
    pub balance: i128,            // 16 bytes
    pub start_time: u64,          // 8 bytes
    pub end_time: u64,            // 8 bytes
    pub is_active: bool,          // 1 byte
}
// Total: ~113 bytes + encoding overhead ≈ 200 bytes on-chain
```

### Operation Complexity

| Operation | Storage Access | Typical Instructions | Notes |
|-----------|---------------|---------------------|-------|
| `create_stream` | 2 writes | < 500K | Creates stream + updates counter |
| `start_stream` | 1 read, 1 write | < 300K | Updates stream state |
| `stop_stream` | 1 read, 1 write | < 300K | Updates stream state |
| `settle_stream` | 1 read, 1 write | < 400K | Includes arithmetic |
| `archive_stream` | 1 read, 1 delete | < 200K | Removes from storage |
| `get_stream_info` | 1 read | < 100K | Read-only |
| `version` | 0 | < 10K | Returns constant |

All operations remain well within Soroban's resource limits with significant headroom for future enhancements.
