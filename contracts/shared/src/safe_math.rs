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
        self.checked_div(other).unwrap_or_else(|| panic_with_error!(env, SharedError::DivisionByZero))
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
        self.checked_div(other).unwrap_or_else(|| panic_with_error!(env, SharedError::DivisionByZero))
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
        self.checked_div(other).unwrap_or_else(|| panic_with_error!(env, SharedError::DivisionByZero))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    // ======================================================================
    // i128 tests
    // ======================================================================

    #[test]
    fn test_i128_safe_add_success() {
        let env = Env::default();
        assert_eq!(100i128.safe_add(&env, 50i128), 150i128);
        assert_eq!((-100i128).safe_add(&env, 50i128), -50i128);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_i128_safe_add_overflow() {
        let env = Env::default();
        i128::MAX.safe_add(&env, 1i128);
    }

    #[test]
    fn test_i128_safe_sub_success() {
        let env = Env::default();
        assert_eq!(100i128.safe_sub(&env, 50i128), 50i128);
        assert_eq!((-100i128).safe_sub(&env, 50i128), -150i128);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn test_i128_safe_sub_underflow() {
        let env = Env::default();
        i128::MIN.safe_sub(&env, 1i128);
    }

    #[test]
    fn test_i128_safe_mul_success() {
        let env = Env::default();
        assert_eq!(100i128.safe_mul(&env, 3i128), 300i128);
        assert_eq!((-10i128).safe_mul(&env, 5i128), -50i128);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_i128_safe_mul_overflow() {
        let env = Env::default();
        i128::MAX.safe_mul(&env, 2i128);
    }

    #[test]
    fn test_i128_safe_div_success() {
        let env = Env::default();
        assert_eq!(100i128.safe_div(&env, 10i128), 10i128);
        assert_eq!((-100i128).safe_div(&env, 5i128), -20i128);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_i128_safe_div_by_zero() {
        let env = Env::default();
        100i128.safe_div(&env, 0i128);
    }

    // ======================================================================
    // u64 tests
    // ======================================================================

    #[test]
    fn test_u64_safe_add_success() {
        let env = Env::default();
        assert_eq!(100u64.safe_add(&env, 50u64), 150u64);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_u64_safe_add_overflow() {
        let env = Env::default();
        u64::MAX.safe_add(&env, 1u64);
    }

    #[test]
    fn test_u64_safe_sub_success() {
        let env = Env::default();
        assert_eq!(100u64.safe_sub(&env, 50u64), 50u64);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn test_u64_safe_sub_underflow() {
        let env = Env::default();
        0u64.safe_sub(&env, 1u64);
    }

    #[test]
    fn test_u64_safe_mul_success() {
        let env = Env::default();
        assert_eq!(100u64.safe_mul(&env, 3u64), 300u64);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_u64_safe_mul_overflow() {
        let env = Env::default();
        u64::MAX.safe_mul(&env, 2u64);
    }

    #[test]
    fn test_u64_safe_div_success() {
        let env = Env::default();
        assert_eq!(100u64.safe_div(&env, 10u64), 10u64);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_u64_safe_div_by_zero() {
        let env = Env::default();
        100u64.safe_div(&env, 0u64);
    }

    // ======================================================================
    // u32 tests
    // ======================================================================

    #[test]
    fn test_u32_safe_add_success() {
        let env = Env::default();
        assert_eq!(100u32.safe_add(&env, 50u32), 150u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_u32_safe_add_overflow() {
        let env = Env::default();
        u32::MAX.safe_add(&env, 1u32);
    }

    #[test]
    fn test_u32_safe_sub_success() {
        let env = Env::default();
        assert_eq!(100u32.safe_sub(&env, 50u32), 50u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn test_u32_safe_sub_underflow() {
        let env = Env::default();
        0u32.safe_sub(&env, 1u32);
    }

    #[test]
    fn test_u32_safe_mul_success() {
        let env = Env::default();
        assert_eq!(100u32.safe_mul(&env, 3u32), 300u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #9)")]
    fn test_u32_safe_mul_overflow() {
        let env = Env::default();
        u32::MAX.safe_mul(&env, 2u32);
    }

    #[test]
    fn test_u32_safe_div_success() {
        let env = Env::default();
        assert_eq!(100u32.safe_div(&env, 10u32), 10u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #13)")]
    fn test_u32_safe_div_by_zero() {
        let env = Env::default();
        100u32.safe_div(&env, 0u32);
    }
}
