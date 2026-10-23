# soroban-multisig-timelock

Production-ready multi-signature treasury and streaming timelock smart contract for Stellar Soroban with emergency veto controls and storage TTL management.

## Capabilities
- **M-of-N Threshold Treasury**: Multi-signature proposal creation, cryptographic approval collection (`require_auth()`), and execution.
- **Configurable Timelock Delay**: Prevents instant drainage; requires configurable delay periods (e.g. 48 hours) for community review.
- **Linear Streaming Disbursements**: Continuous vesting and withdrawal schedules for grant distributions and payroll.
- **Emergency Guardian Veto**: Immediate cancellation controls for security councils to stop zero-day exploits or rogue transactions.
- **State Rent TTL Bump Management**: Proactively extends Instance and Persistent storage lifetimes (`extend_ttl`) to ensure long-term availability under Stellar Soroban storage rent rules.
