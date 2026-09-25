import os
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
    
    original = content

    # Replace checked_add(X).expect(Y) with safe_add(&env, X)
    content = re.sub(r'\.checked_add\(([^)]+)\)\s*\.\s*expect\([^)]+\)', r'.safe_add(&env, \1)', content)
    content = re.sub(r'\.checked_add\(([^)]+)\)\s*\.\s*unwrap_or\([^)]+\)', r'.safe_add(&env, \1)', content)
    
    content = re.sub(r'\.checked_sub\(([^)]+)\)\s*\.\s*expect\([^)]+\)', r'.safe_sub(&env, \1)', content)
    content = re.sub(r'\.checked_sub\(([^)]+)\)\s*\.\s*unwrap_or\([^)]+\)', r'.safe_sub(&env, \1)', content)

    content = re.sub(r'\.checked_mul\(([^)]+)\)\s*\.\s*expect\([^)]+\)', r'.safe_mul(&env, \1)', content)
    content = re.sub(r'\.checked_mul\(([^)]+)\)\s*\.\s*unwrap_or\([^)]+\)', r'.safe_mul(&env, \1)', content)
    
    content = re.sub(r'\.checked_div\(([^)]+)\)\s*\.\s*expect\([^)]+\)', r'.safe_div(&env, \1)', content)
    content = re.sub(r'\.checked_div\(([^)]+)\)\s*\.\s*unwrap_or\([^)]+\)', r'.safe_div(&env, \1)', content)

    # Some fold functions use expect
    # e.g., acc.checked_add(m.amount).expect("Amount overflow")
    # But wait, fold often doesn't have `env`.
    # Let's verify fold first. If it's a lambda without `env`, `safe_add(&env, ...)` will fail to compile.
    # We will leave fold alone or fix manually if needed.
    # For now, let's just do a blanket replace and fix compiler errors later.

    # Also we need to add `use shared::SafeMath;` to the file if it's modified.
    if content != original:
        if "use shared::SafeMath;" not in content:
            if "use shared::{" in content:
                content = content.replace("use shared::{", "use shared::{SafeMath, ", 1)
            else:
                content = "use shared::SafeMath;\n" + content

    with open(filepath, 'w') as f:
        f.write(content)
    
    print(f"Processed {filepath}")

if __name__ == "__main__":
    process_file("escrow/src/lib.rs")
    process_file("contracts/staking/src/lib.rs")
    process_file("contracts/treasury/src/lib.rs")
