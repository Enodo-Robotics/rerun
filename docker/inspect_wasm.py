#!/usr/bin/env python3
"""Dump the table section and `__wbindgen_export_*` exports of a wasm file.

Healthy wasm-bindgen output has:
  table[0] = funcref, growable (no max, or large max)
  table[1] = externref
  export __wbindgen_export_1 -> table[1] (externref)
  export __wbindgen_export_6 -> table[0] (funcref)

The binaryen 105 -O2 bug rewrites both export targets to table[0], which the
JS shim then uses to .set() externref objects into — crashing at runtime.
"""
import sys
from pathlib import Path


def read_uleb(buf, i):
    r, s = 0, 0
    while True:
        b = buf[i]
        i += 1
        r |= (b & 0x7F) << s
        if b & 0x80 == 0:
            return r, i
        s += 7


def parse(buf):
    assert buf[:4] == b"\0asm", "not a wasm file"
    i = 8  # past magic + version
    tables = []
    exports = []
    while i < len(buf):
        section_id = buf[i]
        i += 1
        size, i = read_uleb(buf, i)
        end = i + size
        if section_id == 4:  # table
            count, i = read_uleb(buf, i)
            for _ in range(count):
                reftype = buf[i]
                i += 1
                flags = buf[i]
                i += 1
                min_, i = read_uleb(buf, i)
                max_ = None
                if flags & 1:
                    max_, i = read_uleb(buf, i)
                tables.append((reftype, min_, max_))
        elif section_id == 7:  # export
            count, i = read_uleb(buf, i)
            for _ in range(count):
                name_len, i = read_uleb(buf, i)
                name = buf[i : i + name_len].decode("utf-8", "replace")
                i += name_len
                kind = buf[i]
                i += 1
                idx, i = read_uleb(buf, i)
                exports.append((name, kind, idx))
        i = end
    return tables, exports


def main(path):
    buf = Path(path).read_bytes()
    tables, exports = parse(buf)

    print(f"Tables ({len(tables)}):")
    reftype_name = {0x70: "funcref", 0x6F: "externref"}
    for idx, (rt, mn, mx) in enumerate(tables):
        print(
            f"  table[{idx}]: type={reftype_name.get(rt, hex(rt))} min={mn} max={mx}"
        )

    kind_name = {0: "func", 1: "table", 2: "memory", 3: "global"}

    print("\nAll table exports:")
    for name, kind, idx in exports:
        if kind == 1:
            rt = (
                reftype_name.get(tables[idx][0], hex(tables[idx][0]))
                if idx < len(tables)
                else "?"
            )
            print(f"  {name} -> table[{idx}]  ({rt})")

    print("\n__wbindgen* exports (any):")
    wb = [e for e in exports if "__wbindgen" in e[0] or "wbindgen" in e[0]]
    for name, kind, idx in sorted(wb):
        tag = kind_name.get(kind, f"kind={kind}")
        extra = ""
        if kind == 1 and idx < len(tables):
            extra = f"  ({reftype_name.get(tables[idx][0], hex(tables[idx][0]))})"
        print(f"  {name} -> {tag}[{idx}]{extra}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "re_viewer_bg.wasm")
