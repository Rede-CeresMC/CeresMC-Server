"""Furnace test (python tools/furnace_test.py): a blast furnace with raw iron and coal lights up,
shows its progress and makes an iron ingot; nothing can be put into the result slot."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, svarint, read_uvarint, string

STONE = block_id("minecraft:stone")
BLAST = block_id("minecraft:blast_furnace")
POS = (274, 70, 266)
HOTBAR, INPUT, FUEL, RESULT = 28, 45, 24, 26

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def give(c, what):
    n = len(c.received)
    command(c, "/give @s " + what)
    c.pump(3.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def slot(container, s):
    return bytes([container, 0, s]) + struct.pack("<i", 0)

counter = [-100]
def request(c, actions):
    counter[0] -= 2
    body = svarint(counter[0]) + uvarint(len(actions))
    for ty, data in actions:
        body += uvarint(ty) + bytes(1) + data
    body += uvarint(0) + struct.pack("<i", 0)
    n = len(c.received)
    c.send(uvarint(0x93) + uvarint(1) + body)
    c.pump(3.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
    resp = [d for p, d in c.received[n:] if p == 0x94]
    return bool(resp) and resp[-1][1] == 0

def take(count, src, dst):
    return (0, bytes([count]) + src + dst)

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
            out.append((iid, cnt))
        return out
    return None

c = Client("BotFurnace")
c.join()
c.move(274.5, 71.62, 262.5)
c.pump(0.5)
command(c, "/gamemode creative")
below = (POS[0], POS[1] - 1, POS[2])
c.break_block(POS)
c.place_block(below, 1, STONE)
c.pump(0.4)
c.place_block(below, 1, BLAST)
c.pump(1.0)
command(c, "/gamemode survival")
command(c, "/clear")
give(c, "raw_iron 1")
give(c, "coal 1")

n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 27, f"clicking a blast furnace opens its window ({opens and opens[-1][:2]})")
win = opens[-1][0] if opens else 0
check(request(c, [take(1, slot(HOTBAR, 0), slot(INPUT, 0))]), "raw iron goes into the input slot")
check(not request(c, [take(1, slot(HOTBAR, 1), slot(RESULT, 2))]), "nothing can be put into the result slot")
n = len(c.received)
check(request(c, [take(1, slot(HOTBAR, 1), slot(FUEL, 1))]), "coal goes into the fuel slot")
c.pump(1.5)
lit = block_id("minecraft:lit_blast_furnace")
check(any(p == POS and b != BLAST for p, b in update_blocks(c)[-20:]), "the blast furnace lights up")
check(any(p == 0x33 for p, _ in c.received[n:]), "progress is sent (ContainerSetData)")
c.pump(5.0, lambda: (contents(c, win, n) or [(0, 0)] * 3)[2][1] == 1)
items = contents(c, win, n)
check(items is not None and items[2][1] == 1 and items[0][1] == 0, f"after 5 s the iron is smelted ({items})")
check(request(c, [take(1, slot(RESULT, 2), slot(HOTBAR, 2))]), "the ingot can be taken out")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
