# Indexing Strategy

## Payer Stream Index (Capped)

Each payer maintains a capped list of recent stream IDs on-chain.

- **Max size**: 50
- **Overflow policy**: FIFO (Oldest removed when cap exceeded)
- **Data Structure**: `Map<Address, Vec<u32>>`
- **Purpose**: Provides lightweight UI support for retrieving a user's recent streams directly from the ledger without needing a separate indexer/subgraph.

### Implementation Details

The index is updated automatically during `create_stream()`. 

- **Storage Type**: Persistent storage is used to ensure the index remains available as long as the payer is active.
- **Gas Considerations**: 
  - `O(1)` for adding a new ID when under the cap.
  - `O(N)` where `N=50` for shifting elements when the cap is reached. This is well within the gas limits for a single transaction.

### Limitations

- **Recent only**: Only the 50 most recent streams created by a payer are retained in this index. Older streams will be rotated out.
- **Not Source of Truth**: The index is an optimization for discovery. The core stream data remains the definitive source of truth.
