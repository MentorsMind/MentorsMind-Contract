#![no_std]
use soroban_sdk::{symbol_short, Symbol};

/// Role constants for RBAC system
/// These are used across all contracts to check permissions
/// Using abbreviations to fit within Soroban's 9-character symbol limit
pub const ROLE_SUPER_ADMIN: Symbol = symbol_short!("SUP_ADMIN");
pub const ROLE_TREASURY_ADMIN: Symbol = symbol_short!("TRS_ADMIN");
pub const ROLE_STAKING_ADMIN: Symbol = symbol_short!("STK_ADMIN");
pub const ROLE_GOVERNANCE_ADMIN: Symbol = symbol_short!("GOV_ADMIN");
pub const ROLE_ORACLE_FEEDER: Symbol = symbol_short!("ORCL_FEED");
pub const ROLE_ARBITRATOR: Symbol = symbol_short!("ARBITR");
