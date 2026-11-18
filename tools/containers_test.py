"""Chest test (python tools/containers_test.py [check]).

Without arguments: place a chest, open it, put stone in, close, reopen and see the stone, break a
second chest and see its contents drop. Leaves a filled chest at KEEP for the "check" run.
With "check" (after a server restart): the chest at KEEP still holds the stone.
"""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, svarint, read_uvarint, string

STONE = block_id("minecraft:stone")
CHEST = block_id("minecraft:chest")
KEEP = (266, 70, 266)
SPARE = (268, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + b"\x00" + string(""))

def slot(container, s, stack_id=0):
    return bytes([container]) + b"\x00" + bytes([s]) + struct.pack("<i", stack_id)

def request(req_id, actions):
    body = svarint(req_id) + uvarint(len(actions))
    for ty, data in actions:
        body += uvarint(ty) + b"\x00" + data
    body += uvarint(0) + struct.pack("<i", 0)
    return uvarint(0x93) + uvarint(1) + body

def read_item(d, p):
    iid, count = struct.unpack("<hH", d[p:p + 4]); p += 4
    _, p = read_uvarint(d, p)
    if d[p]:
        p += 1; _, p = read_uvarint(d, p)
    else:
        p += 1
    _, p = read_uvarint(d, p)
    n, p = read_uvarint(d, p); p += n
    return (iid, count), p

def contents(c, window):
    """Items of the last InventoryContent for `window`."""
    for pid, d in reversed(c.received):
        if pid != 0x31:
            continue
        w, p = read_uvarint(d, 0)
        if w != window:
            continue
        n, p = read_uvarint(d, p)
        items = []
        for _ in range(n):
            it, p = read_item(d, p)
            items.append(it)
        return items
    return None

def open_chest(c, pos):
    """Click the chest; returns its window id (or None)."""
    n = len(c.received)
    c.place_block(pos, 1, 0)
    c.pump(4.0, lambda: any(p == 0x2e for p, _ in c.received[n:]) and any(p == 0x31 for p, _ in c.received[n:]))
    opens = [d for p, d in c.received[n:] if p == 0x2e]
    return opens[-1][0] if opens else None

def close_chest(c, window):
    c.send(uvarint(0x2f) + bytes([window, 0]) + b"\x00")
    c.pump(1.0)

def place_chest(c, pos):
    below = (pos[0], pos[1] - 1, pos[2])
    c.break_block(pos)
    c.place_block(below, 1, STONE)  # air below: becomes a stone support
    c.pump(0.5)
    c.place_block(below, 1, CHEST)
    c.pump(4.0, lambda: (pos, CHEST) in update_blocks(c) or any(p2 == pos and b != 0 for p2, b in update_blocks(c)))

c = Client("BotChest")
c.join()
c.move(266.5, 71.62, 262.5)
c.pump(0.5)

if len(sys.argv) > 1 and sys.argv[1] == "check":
    win = open_chest(c, KEEP)
    items = contents(c, win) if win else None
    check(items is not None and any(it[1] == 64 for it in items), f"chest contents survived the restart ({items and [i for i in items if i[1]]})")
else:
    command(c, "/gamemode creative")
    c.pump(0.5)
    place_chest(c, KEEP)
    win = open_chest(c, KEEP)
    check(win is not None, f"clicking a chest opens it (window {win})")
    check(win is not None and contents(c, win) is not None and len(contents(c, win)) == 27, "the chest window has 27 slots")
    c.received.clear()
    # creative: take 64 of creative item #1 straight into chest slot 0
    c.send(request(-2, [(12, uvarint(1) + b"\x01"), (0, b"\x40" + slot(60, 50) + slot(7, 0))]))
    c.pump(4.0, lambda: any(p == 0x94 for p, _ in c.received))
    resp = [d for p, d in c.received if p == 0x94]
    check(resp and resp[-1][1] == 0, "putting items into the chest is accepted")
    close_chest(c, win)
    win2 = open_chest(c, KEEP)
    items = contents(c, win2) if win2 else None
    check(items is not None and items[0][1] == 64, f"reopened chest still holds the items ({items[:1] if items else None})")
    close_chest(c, win2)
    # a second chest with items, broken: the items drop
    place_chest(c, SPARE)
    w3 = open_chest(c, SPARE)
    c.send(request(-4, [(12, uvarint(1) + b"\x01"), (0, b"\x10" + slot(60, 50) + slot(7, 3))]))
    c.pump(2.0, lambda: sum(1 for p, _ in c.received if p == 0x94) >= 2)
    if w3:
        close_chest(c, w3)
    c.received.clear()
    c.break_block(SPARE)
    c.pump(4.0, lambda: any(p == 0x0f for p, _ in c.received))
    check(any(p == 0x0f for p, _ in c.received), "breaking a chest drops what was inside")
    print("  waiting for the autosave...")
    c.pump(6.0)

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
