"""Double chest test (python tools/doublechest_test.py)."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, read_uvarint, string

STONE = block_id("minecraft:stone")
CHEST = block_id("minecraft:chest")
A, B = (284, 70, 266), (285, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def slot(container, s):
    return bytes([container, 0, s]) + struct.pack("<i", 0)

def contents(c, window, since):
    for pid, d in reversed(c.received[since:]):
        if pid != 0x31:
            continue
        w, p = read_uvarint(d, 0)
        if w != window:
            continue
        n, p = read_uvarint(d, p)
        out = []
        for _ in range(n):
            iid, cnt = struct.unpack("<hH", d[p:p + 4]); p += 4
            _, p = read_uvarint(d, p)
            if d[p]:
                p += 1; _, p = read_uvarint(d, p)
            else:
                p += 1
            _, p = read_uvarint(d, p)
            k, p = read_uvarint(d, p); p += k
            out.append(cnt)
        return out
    return None

def open_chest(c, pos):
    n = len(c.received)
    c.place_block(pos, 1, 0)
    c.pump(3.0, lambda: any(p == 0x31 for p, _ in c.received[n:]))
    opens = [d for p, d in c.received[n:] if p == 0x2e]
    win = opens[-1][0] if opens else None
    return win, (contents(c, win, n) if win else None)

def close(c, win):
    c.send(uvarint(0x2f) + bytes([win, 0]) + bytes(1))
    c.pump(0.8)

c = Client("BotChest2")
c.join()
c.move(284.5, 71.62, 262.5)
c.pump(0.5)
command(c, "/gamemode creative")
for pos in (A, B):
    c.break_block(pos)
for pos in (A, B):
    below = (pos[0], pos[1] - 1, pos[2])
    c.place_block(below, 1, STONE)
    c.pump(0.3)
    c.place_block(below, 1, CHEST)
    c.pump(0.6)

win, items = open_chest(c, A)
check(items is not None and len(items) == 54, f"two chests side by side open as one with 54 slots ({items and len(items)})")
n = len(c.received)
body = svarint(-60) + uvarint(2) + uvarint(12) + bytes(1) + uvarint(1) + bytes([1])
body += uvarint(0) + bytes(1) + bytes([16]) + slot(60, 50) + slot(7, 40) + uvarint(0) + struct.pack("<i", 0)
c.send(uvarint(0x93) + uvarint(1) + body)
c.pump(3.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
resp = [d for p, d in c.received[n:] if p == 0x94]
check(resp and resp[-1][1] == 0, "putting items in slot 40 (second half) works")
if win:
    close(c, win)
win, items = open_chest(c, B)
check(items is not None and len(items) == 54 and items[40] == 16, "opening the other half shows the same 54 slots")
if win:
    close(c, win)
n = len(c.received)
c.break_block(B)
c.pump(2.0)
check(any(p == 0x0f for p, _ in c.received[n:]), "breaking the half that holds the items drops them")
win, items = open_chest(c, A)
check(items is not None and len(items) == 27, f"the remaining half is a single chest again ({items and len(items)})")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
