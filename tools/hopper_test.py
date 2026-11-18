"""Hopper (python tools/hopper_test.py): items move from a chest, through a hopper, into the
chest below it."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, read_uvarint, string

STONE = block_id("minecraft:stone")
CHEST = block_id("minecraft:chest")
HOPPER = block_id("minecraft:hopper")
X, Z = 324, 266
TOP, MID, LOW = (X, 72, Z), (X, 71, Z), (X, 70, Z)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

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

def open_at(c, pos):
    n = len(c.received)
    c.place_block(pos, 1, 0)
    c.pump(3.0, lambda: any(p == 0x31 for p, _ in c.received[n:]))
    opens = [d for p, d in c.received[n:] if p == 0x2e]
    win = opens[-1][0] if opens else None
    return win, (contents(c, win, n) if win else None)

def close(c, win):
    c.send(uvarint(0x2f) + bytes([win, 0]) + bytes(1))
    c.pump(0.5)

c = Client("BotHopper")
c.join()
c.move(X + 0.5, 71.62, Z - 2.5)
c.pump(0.5)
for p in (TOP, MID, LOW):
    c.break_block(p)
c.place_block((X, 69, Z), 1, STONE)
c.pump(0.3)
c.place_block((X, 69, Z), 1, CHEST)
c.pump(0.4)
c.sneak(True)
c.pump(0.2)
c.place_block(LOW, 1, HOPPER)      # on top of the lower chest: points down into it
c.pump(0.4)
c.place_block(MID, 1, CHEST)       # chest on top of the hopper
c.pump(0.4)
c.sneak(False)
c.pump(0.4)

win, items = open_at(c, TOP)
check(win is not None, "the upper chest opens")
n = len(c.received)
body = svarint(-80) + uvarint(2) + uvarint(12) + bytes(1) + uvarint(1) + bytes([1])
body += uvarint(0) + bytes(1) + bytes([4]) + slot(60, 50) + slot(7, 0) + uvarint(0) + struct.pack("<i", 0)
c.send(uvarint(0x93) + uvarint(1) + body)
c.pump(2.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
if win:
    close(c, win)
c.pump(5.0)
win, items = open_at(c, LOW)
total = sum(items) if items else 0
check(total == 4, f"all 4 items went through the hopper into the lower chest ({total})")
if win:
    close(c, win)
win, items = open_at(c, TOP)
check(items is not None and sum(items) == 0, "the upper chest is empty")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
