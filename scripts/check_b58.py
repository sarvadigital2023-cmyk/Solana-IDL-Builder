#!/usr/bin/env python3
"""
Checks if a string is a valid Base58 Solana public key (32-44 chars, Base58 alphabet).
Exits 0 if valid, 1 if invalid.
"""
import sys, re

B58_ALPHA = re.compile(r'^[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]{32,44}$')

addr = sys.argv[1] if len(sys.argv) > 1 else ''
if B58_ALPHA.match(addr):
    print(f"VALID: {addr}")
    sys.exit(0)
else:
    invalid_chars = [c for c in addr if not re.match(r'[123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz]', c)]
    print(f"INVALID: '{addr}' (bad chars: {invalid_chars}, len: {len(addr)})")
    sys.exit(1)
