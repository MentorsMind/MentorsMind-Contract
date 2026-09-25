# Monitoring Guide

This guide describes the operational signals that should be watched for the Soroban contracts in this repository.

## What to Monitor

- Contract events for escrow creation, release, dispute, refund, and upgrade activity.
- Authorization failures that may indicate misuse or integration drift.
- Ledger timestamps and TTL extensions for state that should remain alive across normal operation.
- Unexpected spikes in failed calls, especially around payment, dispute, and upgrade paths.

## Recommended Signals

- Event volume by contract and by action.
- Failed transaction count per entrypoint.
- Rate-limit rejections and whitelist changes.
- Storage growth for long-lived records such as upgrade history and open escrows.

## Alerting Notes

- Alert on repeated authorization failures from the same caller.
- Alert when upgrade registration or version checks fail unexpectedly.
- Alert when escrow release or dispute resolution volumes deviate sharply from the baseline.

## Operational Checks

- Confirm the latest deployed contract version matches the expected release.
- Confirm event indexers are still ingesting contract emissions.
- Confirm no critical storage entries are nearing TTL expiry without an expected refresh path.
