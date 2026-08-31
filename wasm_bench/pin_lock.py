import pathlib
import re
import sys


PACKAGES = {
    "0.4.0": """[[package]]
name = "fearless_simd"
version = "0.4.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "76258897e51fd156ee03b6246ea53f3e0eb395d0b327e9961c4fc4c8b2fa151a"
dependencies = [
 "libm",
]

""",
}


def main():
    if len(sys.argv) != 3 or sys.argv[2] not in PACKAGES:
        raise SystemExit("usage: pin_lock.py CARGO_LOCK 0.4.0")

    path = pathlib.Path(sys.argv[1])
    contents = path.read_text()
    pattern = re.compile(
        r'^\[\[package\]\]\nname = "fearless_simd"\n.*?(?=^\[\[package\]\])',
        re.MULTILINE | re.DOTALL,
    )
    updated, replacements = pattern.subn(PACKAGES[sys.argv[2]], contents, count=1)
    if replacements != 1:
        raise SystemExit("expected exactly one fearless_simd package in Cargo.lock")
    path.write_text(updated)


if __name__ == "__main__":
    main()
