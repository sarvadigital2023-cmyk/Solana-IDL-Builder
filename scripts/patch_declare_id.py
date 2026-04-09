#!/usr/bin/env python3
"""
Patches declare_id!("...") in a Rust/Anchor lib.rs file with a new program address.
Usage: python3 patch_declare_id.py <path_to_lib_rs> <new_address>
"""
import re, sys

if len(sys.argv) != 3:
    print(f"Usage: {sys.argv[0]} <lib_rs_path> <new_address>")
    sys.exit(1)

path = sys.argv[1]
new_addr = sys.argv[2]

with open(path, 'r') as f:
    content = f.read()

patched = re.sub(
    r'declare_id!\("[^"]*"\)',
    f'declare_id!("{new_addr}")',
    content
)

with open(path, 'w') as f:
    f.write(patched)

print(f"Patched declare_id! -> {new_addr}")
