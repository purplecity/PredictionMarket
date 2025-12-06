# Integration Tests

This crate contains all integration tests for the prediction market system.

## Structure

```
tests/
├── src/
│   ├── lib.rs          # Test library exports
│   └── test_utils.rs   # Common test utilities
├── tests/              # Integration tests
│   ├── asset_deposit_withdraw.rs
│   ├── asset_create_order.rs
│   ├── asset_cancel_order.rs
│   ├── asset_split_merge.rs
│   └── match_engine_tests.rs
├── README.md
├── ASSET_TESTS_README.md
├── README_TESTS.md
└── TEST_DOCUMENTATION.md
```

## Running Tests

### Run all tests
```bash
cargo test -p tests
```

### Run specific test file
```bash
cargo test -p tests --test asset_deposit_withdraw
cargo test -p tests --test asset_create_order
cargo test -p tests --test asset_cancel_order
cargo test -p tests --test asset_split_merge
cargo test -p tests --test match_engine_tests
```

### Run a specific test
```bash
cargo test -p tests test_usdc_deposit
cargo test -p tests test_create_buy_order
cargo test -p tests test_orderbook_new
```

### Run with output
```bash
cargo test -p tests -- --nocapture
```

## Test Categories

### Asset Tests
- **deposit_withdraw**: Tests for USDC/token deposits and withdrawals
- **create_order**: Tests for order creation (buy/sell)
- **cancel_order**: Tests for order cancellation (full/partial)
- **split_merge**: Tests for USDC↔token conversion

### Match Engine Tests
- **OrderBook tests**: Basic orderbook operations (add, remove, update)
- **MatchEngine tests**: Order matching logic and cross-market matching
- **Price level tests**: Depth snapshot and price level tracking
- **Market order tests**: Market order partial fill behavior
- **Self-trade tests**: Self-trade detection and prevention

## Test Utilities

The `test_utils` module provides:
- `TestEnv`: Database connection pool and test environment setup
- `generate_test_user_id()`: Generate unique user IDs
- `generate_test_event_id()`: Generate unique event IDs
- `parse_decimal()`: Parse decimal strings
- Helper methods for creating test users, events, and querying balances

## Requirements

- PostgreSQL database running locally (default: `127.0.0.1:5432`)
- Database configuration in `deploy/common.env`
- Test database: `prediction_market`

## Notes

- Tests use unique IDs (timestamp-based) to avoid conflicts
- Most tests skip cleanup to prevent connection pool issues
- Each test creates isolated test data
- Tests require a running PostgreSQL instance
