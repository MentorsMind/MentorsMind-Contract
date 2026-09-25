# Optimization Guide

This document summarizes optimization patterns used across the MentorsMind contracts and provides guidance for contributors.

## Fee Caching

- Implement small, explicit caches for repeated, deterministic calculations (e.g., fee = amount * BPS / 10_000).
- Use instance storage with a bounded cache size for common amounts.
- Provide explicit cache invalidation APIs (administrative or programmatic) to recover from parameter changes.

## Yield Caching

- Cache intermediate multipliers or rate lookups that are constant within a transaction or ledger sequence.
- Keep cached values in instance storage where they are safe and low-cost to read.
- Avoid caching values that depend on rapidly-changing state unless invalidation is provided.

## TTL Heuristics

- Use `shared::ttl_utils::should_bump_ttl` and `next_bump_interval` to determine whether to bump TTL now.
- Bump less frequently for long-lived entries and more frequently for short-lived ones.
- Consider cost trade-offs: each bump is a storage write; avoid noisy bumps for large arrays.

## Vector Efficiency

- Prefer iterators and borrowing (`for x in vec.iter()`) to avoid unnecessary copies.
- Use `Vec::push_back` instead of building temporary Rust vectors when interacting with Soroban `Vec<T>`.
- When removing items, prefer in-place rearrangement to repeated allocations.

## Profiling and Benchmarks

- Use `soroban-cli --estimate-gas` and the repo's benchmark harness to measure instruction counts.
- Target critical hot-paths first: token transfer flows, fee calculations, dispute resolution.

## Tests

- Add microbenchmarks that exercise cache hits vs misses.
- Add unit tests verifying cache invalidation behaviour.

*** End of guide
