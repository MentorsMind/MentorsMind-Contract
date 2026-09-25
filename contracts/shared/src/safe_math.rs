use soroban_sdk::{panic_with_error, Env};
use crate::SharedError;

pub trait SafeMath {
    fn safe_add(self, env: &Env, other: Self) -> Self;
    fn safe_sub(self, env: &Env, other: Self) -> Self;
    fn safe_mul(self, env: &Env, other: Self) -> Self;
    fn safe_div(self, env: &Env, other: Self) -> Self;
}

impl SafeMath for i128 {
    fn safe_add(self, env: &Env, other: Self) -> Self {
        self.checked_add(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
    fn safe_sub(self, env: &Env, other: Self) -> Self {
        self.checked_sub(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Underflow))
    }
    fn safe_mul(self, env: &Env, other: Self) -> Self {
        self.checked_mul(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
    fn safe_div(self, env: &Env, other: Self) -> Self {
        self.checked_div(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow)) // Or DivisionByZero if it existed, but Overflow is suitable enough for Soroban Math limits
    }
}

impl SafeMath for u64 {
    fn safe_add(self, env: &Env, other: Self) -> Self {
        self.checked_add(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
    fn safe_sub(self, env: &Env, other: Self) -> Self {
        self.checked_sub(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Underflow))
    }
    fn safe_mul(self, env: &Env, other: Self) -> Self {
        self.checked_mul(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
    fn safe_div(self, env: &Env, other: Self) -> Self {
        self.checked_div(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
}

impl SafeMath for u32 {
    fn safe_add(self, env: &Env, other: Self) -> Self {
        self.checked_add(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
    fn safe_sub(self, env: &Env, other: Self) -> Self {
        self.checked_sub(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Underflow))
    }
    fn safe_mul(self, env: &Env, other: Self) -> Self {
        self.checked_mul(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
    fn safe_div(self, env: &Env, other: Self) -> Self {
        self.checked_div(other).unwrap_or_else(|| panic_with_error!(env, SharedError::Overflow))
    }
}
