# FLAM Device Security Audit Report

> Audit period: 2026-06-14 to 2026-06-21
> Device: FLAM (Lunii-like children's interactive story player)
> Auditor: Thomas (scarlaty) + Claude Code

## 1. Target Description

| Property | Value |
|----------|-------|
| SoC | ESP32 (Xtensa LX6, dual-core) |
| RAM | DRAM 0x3FC88000 - 0x3FD00000 (~480 KB) |
| Display | 320x240, LVGL 8.3 |
| Scripting | Lua 5.4 (embedded) |
| Filesystem | SPIFFS/LittleFS (flat, no directories) |
| Input | Rotary encoder (wheel + click) |
| Connectivity | USB (mass storage for story loading) |
| Story format | `.plain.pk` (ZIP store, compression=0) |

The FLAM device runs interactive children's stories written in Lua. Stories are loaded via USB as `.plain.pk` archives containing Lua scripts, images (.lif format), and audio (.mp3). The Lua runtime has access to LVGL UI bindings and a limited set of firmware APIs.

## 2. Attack Surface

A malicious story `.plain.pk` loaded via USB can execute arbitrary Lua code within the device sandbox. The sandbox restricts standard Lua libraries:

| Library | Status |
|---------|--------|
| `io` | **nil** (blocked) |
| `os` | **nil** (blocked) |
| `debug` | **nil** (blocked) |
| `package` | **nil** (blocked) |
| `lv.*` | Full LVGL bindings available |
| `audio.*` | Audio playback API |
| `progression.*` | Save/load to device filesystem |
| `state.*` | In-memory state management |
| `require()` | Module loader (relative to `script/` dir) |
| `print()` | Serial output (visible on UART) |

## 3. Confirmed Vulnerabilities

### 3.1 Use-After-Free — Heap Corruption (CWE-416) — HIGH

**Status: CONFIRMED on device**

`lv_obj_del()` frees the C-side LVGL object but does not nullify the Lua userdata pointer. Subsequent operations on the dangling Lua reference access freed memory.

**Struct-into-struct overlap (confirmed stable):**
1. Create a label (spy), delete it — struct memory freed (~64B)
2. Create a new label (victim) — LVGL allocator reuses the freed block
3. Operations on spy now affect victim's memory

**Device results:**
- Target address: `0x3FCB0E10`
- Overwrite sizes tested: 8B, 16B, 32B, 64B, 128B, 256B, **512B** — all stable
- `lv.label.set_text(spy, payload)` writes into victim's struct
- `lv.label.get_text(spy)` reads victim's text buffer content

**Text-buffer-into-struct overlap (confirmed once, unstable):**
1. Create helper label with text "H"
2. Create probe label, delete it (struct freed)
3. `set_text(helper, craft)` — text buffer may land in freed struct
4. Craft contains a target address at offset 36 (text pointer)
5. `get_text(probe)` follows crafted pointer → **arbitrary memory read**

Successfully read 11B from address `0x3FCB0C68` on first attempt. Subsequent attempts produced `0x3FCB0D40` with 402B read, then 441B read from `0x3FCB0192-0x3FCB034A`.

**Limitation:** The text-buffer-into-struct overlap depends on LVGL's LIFO best-fit allocator state. It only works when the heap is sufficiently fragmented. Artificial fragmentation (warmup alloc/free patterns) consistently fails — only natural fragmentation from real UI operations produces the right heap state. Calibrated struct size: **sz=63** (not the expected ~75 from `lv_label_t` layout analysis).

**Root cause in code:**
- `src/bindings/lua_lv.h:94-97` — `lua_lv_check_obj()` returns pointer without null check
- `src/bindings/lua_lv_obj.c:21-25` — `l_obj_del()` does not set `*ud = NULL` after delete

**Safe getters on freed objects:** `get_text`, `get_state`, `get_child_cnt`, `get_scroll_y`
**Crashing getters:** `get_width`, `get_height`, `get_x` (call `lv_obj_update_layout` which traverses freed parent)

### 3.2 require() Path Traversal — Information Leak (CWE-22 / CWE-200) — MEDIUM

**Status: CONFIRMED on device**

`require("../../etc/library/list")` resolves relative to `script/` and leaks filesystem paths in error messages:
```
PATH: /../etc/library
```

`require("../main")` successfully loads and executes the main script of the parent story directory, returning `LOADED boolean`. This enables **cross-story code execution** — a malicious story can execute code from other installed stories.

### 3.3 Debug Functions in Production (CWE-489) — LOW

**Status: CONFIRMED on device**

Two debug functions are accessible in the Lua global scope on the production device:
- `print_mem_stat()` — dumps memory statistics (heap usage, free blocks)
- `break_gdb` — GDB breakpoint trigger (found but not called during audit to avoid crash)

25 custom globals found on device: `setup`, `Start`, `LoadBranchA/B/CFunction`, `IntroCard`, `Display_fresques`, `EndStory`, `RestartStory`, `goto_library`, `ButtonTheme`, `audio`, `context_menu`, `progression`, `state`, `progress`, etc.

### 3.4 Integer Overflow in LVGL Text Rendering (CWE-190) — LOW

**Status: CONFIRMED in emulator, not tested on device**

`libs/lvgl/src/misc/lv_txt.c:111` — integer overflow in text height calculation when label text exceeds ~65KB. Triggerable via `lv.label.set_text(label, string.rep("A", 70000))`.

## 4. Negative Findings (Blocked Attacks)

### 4.1 Path Traversal via progression.save/load — BLOCKED

`progression.save("../../tmp/audit", data)` writes but does NOT escape the story's save directory. The flat filesystem (SPIFFS/LittleFS) ignores `../` path components. Files are always created as flat names in `D:/usr/0/{story-uuid}/`.

Similarly, `progression.load("../../etc/wifi/config")` returns empty table, not actual config data.

### 4.2 Sandbox Escape via io/os/debug — BLOCKED

All dangerous Lua standard libraries are nil. No `dofile`, no `loadfile`, no `os.execute`. The sandbox is strict.

### 4.3 Audio Path Traversal — BLOCKED

`audio.load(1, "../../etc/library/list", nil)` fails without leaking information.

## 5. Data Exfiltration Technique

The only way to write persistent data from within the Lua sandbox is `progression.save(key, table)`:
- Writes to `D:/usr/0/{story-uuid}/{key}.save`
- Custom serialization format (not Lua): `#   key$value\anotherkey$value`
- Hex data must be saved as flat strings: `{ d = "ADDR:HEX|ADDR:HEX|..." }`
- Table keys/values containing binary data get mangled by the serializer
- Files readable over USB when device connected to PC
- **No RTC** — all file dates show as 1980

## 6. Technical Details

### LVGL Allocator Behavior
- LIFO best-fit: freed blocks are reused by the next allocation of same size
- `lv_label_t` extends `lv_obj_t` with `char *text` pointer at offset 36
- `set_text` always calls `lv_mem_realloc` even for same-length strings, moving the buffer
- Arbitrary read requires full create/delete cycle per read (cannot reuse probe)

### Device Constraints
- Watchdog reboots if too many LVGL operations per RTOS tick
- Must yield via timer (150-200ms between operations)
- Maximum 1-4 reads per timer tick before watchdog triggers
- Resume mechanism via `hunt_cursor.save` needed for multi-reboot scans

### Arbitrary Read Craft Format
```
[0x01 * 36 bytes (padding)] [target_addr LE 4 bytes] [0x01 * (sz-40) bytes]
```
Where `sz` is the struct size (63 on this device). The target address at offset 36 overwrites the `text*` pointer in `lv_label_t`.

## 7. Tools Developed

| File | Purpose |
|------|---------|
| `stories/histoire-malveillante.plain/` | Main audit story with all test phases |
| `tests/lua/poc_use_after_free.lua` | UAF proof-of-concept |
| `tests/lua/poc_memory_leak.lua` | Memory leak detection |
| `tests/lua/poc_struct_dump.lua` | LVGL struct layout analysis |
| `tests/lua/poc_residual_read.lua` | Residual data read after free |
| `tests/lua/poc_write_corruption.lua` | Write corruption via UAF |
| `tests/lua/poc_story_test.lua` | Story-level integration test |
| `tests/lua/fuzz_lvgl.lua` | LVGL fuzzer (buffer overflow, UAF, integer overflow) |
| `flam-device-inventory.md` | Device filesystem inventory |

## 8. Recommendations

1. **NULL after free** — Set `*ud = NULL` in `l_obj_del()` and check in `lua_lv_check_obj()`. This eliminates the entire UAF class.
2. **Sandbox require()** — Restrict `require()` to only load from `script/` without `..` components. Filter path traversal in the module loader.
3. **Remove debug functions** — Strip `print_mem_stat` and `break_gdb` from production firmware.
4. **Fix integer overflow** — Add bounds check in `lv_txt.c:111` for text height calculation.

## 9. Remaining Work

- **Arbitrary read stabilization** — The text-buffer-into-struct overlap is heap-state-dependent. Further research needed on reliable heap shaping for the ESP32 LVGL allocator.
- **DRAM/Flash string scanning** — String hunter scans 0x3FC88000-0x3FD00000 (DRAM) and 0x3C000000-0x3C100000 (Flash rodata) but has not yet extracted strings due to calibration failures.
- **Firmware binary extraction** — If arbitrary read stabilizes, dump Flash for offline analysis.
