"""Cauldron (python tools/cauldron_test.py): a water bucket fills it, a bottle takes water."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string, read_uvarint

STONE = block_id("minecraft:stone")
CAULDRON = block_id("minecraft:cauldron")
POS = (356, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def hold(c, what):
    command(c, "/clear")
    n = len(c.received)
    command(c, "/give @s " + what)
    c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def events(c, since):
    out = []
    for pid, d in c.received[since:]:
        if pid == 0x19:
            v, p = read_uvarint(d, 0)
            out.append((v >> 1) ^ -(v & 1))
    return out

def block_at(c, since):
    seen = [b for p, b in update_blocks(type("X", (), {"received": c.received[since:]})()) if p == POS]
    return seen[-1] if seen else None

c = Client("BotCauldron")
c.join()
c.move(POS[0] + 0.5, 71.62, POS[2] - 2.5)
c.pump(0.5)
command(c, "/gamemode creative")
c.break_block(POS)
c.place_block((POS[0], 69, POS[2]), 1, STONE)
c.pump(0.3)
c.place_block((POS[0], 69, POS[2]), 1, CAULDRON)
c.pump(0.6)
command(c, "/gamemode survival")

hold(c, "water_bucket 1")
n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(1.0)
check(3506 in events(c, n), "a water bucket fills the cauldron (sound)")
full = block_at(c, n)
check(full is not None and full != CAULDRON, "the cauldron shows the water")

hold(c, "glass_bottle 1")
n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(1.0)
check(3507 in events(c, n), "a glass bottle takes water")
check(block_at(c, n) not in (None, full), "the water level goes down")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
