#!/usr/bin/env python3
"""Pack the standard PNG sizes from an iconset into a modern ICNS file."""

from pathlib import Path
import struct
import sys


def entry(kind: bytes, payload: bytes) -> bytes:
    return kind + struct.pack(">I", len(payload) + 8) + payload


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: build-icns.py ICONSET OUTPUT")

    iconset = Path(sys.argv[1])
    output = Path(sys.argv[2])
    # These types are the PNG-backed 1x/2x sizes understood by macOS.
    sources = (
        (b"icp4", "icon_16x16.png"),
        (b"icp5", "icon_32x32.png"),
        (b"ic07", "icon_128x128.png"),
        (b"ic08", "icon_128x128@2x.png"),
        (b"ic09", "icon_256x256@2x.png"),
        (b"ic10", "icon_512x512@2x.png"),
    )
    entries = []
    for kind, name in sources:
        payload = (iconset / name).read_bytes()
        if payload[:8] != b"\x89PNG\r\n\x1a\n":
            raise SystemExit(f"{name} is not a PNG")
        entries.append(entry(kind, payload))

    body = b"".join(entries)
    output.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)


if __name__ == "__main__":
    main()
