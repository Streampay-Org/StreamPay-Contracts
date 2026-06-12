# Stellar CLI Cheatsheet

A condensed reference for the most common `stellar-cli` (formerly
`soroban-cli`) commands when working with `streampay-contracts`. Replace
`$CONTRACT_ID`, `$PAYER`, `$RECIPIENT`, and any token addresses with the
real values for your environment.

## One-time setup

```bash
# Install the CLI (binary release recommended)
curl -sSf https://soroban.stellar.org/install.sh | bash

# Configure a network
stellar network add testnet \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015"

# Create a local identity
stellar keys generate alice --network testnet
```

## Deploy

```bash
./scripts/build.sh
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/streampay_contracts.wasm \
  --network testnet \
  --source alice
```

The command prints the new `$CONTRACT_ID`. Save it.

## Invoke

```bash
# create_stream
stellar contract invoke \
  --id $CONTRACT_ID --network testnet --source alice \
  -- create_stream \
  --payer $PAYER --recipient $RECIPIENT \
  --rate_per_second 100 --initial_balance 10000 \
  --memo "invoice-42" --end_time 0

# start_stream
stellar contract invoke \
  --id $CONTRACT_ID --network testnet --source alice \
  -- start_stream --stream_id 1

# settle_stream
stellar contract invoke \
  --id $CONTRACT_ID --network testnet --source alice \
  -- settle_stream --stream_id 1

# withdraw_stream
stellar contract invoke \
  --id $CONTRACT_ID --network testnet --source recipient \
  -- withdraw_stream --stream_id 1

# read-only metadata
stellar contract invoke \
  --id $CONTRACT_ID --network testnet --source alice \
  -- get_stream_info --stream_id 1
```

## Useful flags

| Flag | When to use |
|---|---|
| `--fee 1000000` | Bump fee on a busy network. |
| `--cost` | Print resource usage so you can right-size production fees. |
| `--send=yes` | Skip the simulate-only step when you are sure. |
| `--instructions 100000000` | Raise instruction limit for `batch_settle` near the cap. |
