import os
import sys

def check_file(path):
    with open(path, 'r') as f:
        lines = f.readlines()
    
    errors = 0
    prev_write = None
    prev_line_num = -1
    
    for i, line in enumerate(lines):
        line_stripped = line.strip()
        if not line_stripped:
            continue
            
        # Consider any env.storage().*.set(...) as a storage write
        if line_stripped.startswith("env.storage().") and ".set(" in line_stripped:
            if prev_write == line_stripped:
                print(f"::error file={path},line={i+1}::Duplicate consecutive storage write found in {path} at lines {prev_line_num} and {i+1}: {line_stripped}")
                errors += 1
            prev_write = line_stripped
            prev_line_num = i + 1
        else:
            prev_write = None
            
    return errors

def main():
    errors = 0
    # Walk through all directories, ignoring target/ and other build artifacts
    for root, dirs, files in os.walk('.'):
        if 'target' in dirs:
            dirs.remove('target')
        if '.git' in dirs:
            dirs.remove('.git')
        
        for file in files:
            if file.endswith('.rs'):
                path = os.path.join(root, file)
                errors += check_file(path)
    
    if errors > 0:
        print(f"Found {errors} duplicate storage write(s).")
        sys.exit(1)
    else:
        print("No duplicate consecutive storage writes found.")
        sys.exit(0)

if __name__ == '__main__':
    main()
