"""Prepare the supplied MuriArc mark for app-icon generation.

Keep every RGB pixel intact and preserve the white interior of the open mark.
Only the outer square canvas becomes transparent via an antialiased circle.
This intentionally uses only Python's standard library for WSL portability.
"""

from __future__ import annotations

import binascii
import math
import struct
import sys
import zlib
from pathlib import Path

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def read_rgb_png(path: Path) -> tuple[int, int, bytearray]:
    payload = path.read_bytes()
    if not payload.startswith(PNG_SIGNATURE):
        raise ValueError("not a PNG file")
    offset = len(PNG_SIGNATURE)
    compressed = bytearray()
    width = height = 0
    while offset < len(payload):
        length = struct.unpack(">I", payload[offset : offset + 4])[0]
        kind = payload[offset + 4 : offset + 8]
        data = payload[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if kind == b"IHDR":
            width, height, bit_depth, color_type, compression, filtering, interlace = struct.unpack(">IIBBBBB", data)
            if (bit_depth, color_type, compression, filtering, interlace) != (8, 2, 0, 0, 0):
                raise ValueError("expected a non-interlaced 8-bit RGB PNG")
        elif kind == b"IDAT":
            compressed.extend(data)
        elif kind == b"IEND":
            break

    raw = zlib.decompress(compressed)
    stride = width * 3
    decoded = bytearray(height * stride)
    previous = bytearray(stride)
    cursor = 0
    for y in range(height):
        filter_type = raw[cursor]
        cursor += 1
        scanline = bytearray(raw[cursor : cursor + stride])
        cursor += stride
        for x in range(stride):
            left = scanline[x - 3] if x >= 3 else 0
            up = previous[x]
            upper_left = previous[x - 3] if x >= 3 else 0
            if filter_type == 1:
                scanline[x] = (scanline[x] + left) & 255
            elif filter_type == 2:
                scanline[x] = (scanline[x] + up) & 255
            elif filter_type == 3:
                scanline[x] = (scanline[x] + ((left + up) >> 1)) & 255
            elif filter_type == 4:
                estimate = left + up - upper_left
                distances = (abs(estimate - left), abs(estimate - up), abs(estimate - upper_left))
                predictor = (left, up, upper_left)[distances.index(min(distances))]
                scanline[x] = (scanline[x] + predictor) & 255
            elif filter_type != 0:
                raise ValueError(f"unsupported PNG filter {filter_type}")
        decoded[y * stride : (y + 1) * stride] = scanline
        previous = scanline
    return width, height, decoded


def chunk(kind: bytes, data: bytes) -> bytes:
    checksum = binascii.crc32(kind + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)


def write_rgba_png(path: Path, width: int, height: int, pixels: bytearray) -> None:
    stride = width * 4
    raw = b"".join(b"\0" + bytes(pixels[y * stride : (y + 1) * stride]) for y in range(height))
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    path.write_bytes(PNG_SIGNATURE + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def resize_rgba(source: bytearray, width: int, height: int, size: int) -> bytearray:
    result = bytearray(size * size * 4)
    for y in range(size):
        source_y = min(height - 1, int((y + 0.5) * height / size))
        for x in range(size):
            source_x = min(width - 1, int((x + 0.5) * width / size))
            source_index = (source_y * width + source_x) * 4
            target_index = (y * size + x) * 4
            result[target_index : target_index + 4] = source[source_index : source_index + 4]
    return result


def write_ico(path: Path, png_payload: bytes, size: int = 256) -> None:
    header = struct.pack("<HHH", 0, 1, 1)
    width_byte = 0 if size == 256 else size
    entry = struct.pack("<BBBBHHII", width_byte, width_byte, 0, 0, 1, 32, len(png_payload), 22)
    path.write_bytes(header + entry + png_payload)


def write_icns(path: Path, png_payload: bytes) -> None:
    element = b"ic10" + struct.pack(">I", 8 + len(png_payload)) + png_payload
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(element)) + element)


def prepare(source: Path, target: Path) -> tuple[int, int, bytearray]:
    width, height, rgb = read_rgb_png(source)
    center_x = (width - 1) / 2
    center_y = (height - 1) / 2
    radius = min(width, height) * 0.468
    feather = max(2.0, min(width, height) * 0.003)
    rgba = bytearray(width * height * 4)
    for y in range(height):
        for x in range(width):
            source_index = (y * width + x) * 3
            target_index = (y * width + x) * 4
            distance = math.hypot(x - center_x, y - center_y)
            alpha = max(0, min(255, round((radius + feather - distance) / feather * 255)))
            rgba[target_index : target_index + 3] = rgb[source_index : source_index + 3]
            rgba[target_index + 3] = alpha
    target.parent.mkdir(parents=True, exist_ok=True)
    write_rgba_png(target, width, height, rgba)
    return width, height, rgba


def create_assets(width: int, height: int, rgba: bytearray, web_dir: Path, tauri_dir: Path, brand_dir: Path) -> None:
    web_dir.mkdir(parents=True, exist_ok=True)
    tauri_dir.mkdir(parents=True, exist_ok=True)
    brand_dir.mkdir(parents=True, exist_ok=True)
    generated: dict[int, Path] = {}
    for size, destination in [
        (32, web_dir / "favicon-32.png"),
        (180, web_dir / "apple-touch-icon.png"),
        (192, web_dir / "pwa-192.png"),
        (512, web_dir / "pwa-512.png"),
        (32, tauri_dir / "32x32.png"),
        (128, tauri_dir / "128x128.png"),
        (256, tauri_dir / "128x128@2x.png"),
        (256, tauri_dir / "icon-256.png"),
        (1024, brand_dir / "logo-1024.png"),
    ]:
        resized = resize_rgba(rgba, width, height, size)
        write_rgba_png(destination, size, size, resized)
        generated[size] = destination
    write_ico(tauri_dir / "icon.ico", (tauri_dir / "icon-256.png").read_bytes())
    write_icns(tauri_dir / "icon.icns", (brand_dir / "logo-1024.png").read_bytes())


if __name__ == "__main__":
    if len(sys.argv) not in (3, 6):
        raise SystemExit("usage: prepare_logo.py SOURCE.png TARGET.png [WEB_DIR TAURI_DIR BRAND_DIR]")
    width, height, rgba = prepare(Path(sys.argv[1]), Path(sys.argv[2]))
    if len(sys.argv) == 6:
        create_assets(width, height, rgba, Path(sys.argv[3]), Path(sys.argv[4]), Path(sys.argv[5]))
