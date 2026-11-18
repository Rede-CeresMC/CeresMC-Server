"""Buckets and flowing water (python tools/liquids_test.py)."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string

STONE = block_id("minecraft:stone")
WATER = block_id("minecraft:water")
AIR = block_id("minecraft:air")
X, Y, Z = 296, 70, 266

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def updates(c, since):
    return update_blocks(type("X", (), {"received": c.received[since:]})())

c = Client("BotWater")
c.join()
c.move(X + 0.5, Y + 1.62, Z - 3.5)
c.pump(0.5)
command(c, "/gamemode creative")
# a 5x5 stone floor at y-1 so the water spreads on it
for dx in range(-2, 3):
    for dz in range(-2, 3):
        c.break_block((X + dx, Y, Z + dz))
        c.place_block((X + dx, Y - 1, Z + dz), 1, STONE)
    c.pump(0.2)
c.pump(1.0)
command(c, "/gamemode survival")
command(c, "/clear")
n0 = len(c.received)
command(c, "/give @s water_bucket 1")
c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n0:]))

n = len(c.received)
c.place_block((X, Y - 1, Z), 1, 0)
c.pump(1.5)
u = updates(c, n)
check(((X, Y, Z), WATER) in u, "a water bucket makes a water source")
slots = [d for p, d in c.received[n:] if p == 0x32]
check(bool(slots), "the bucket in the hand changes (now empty)")
flowing = [p for p, b in u if p != (X, Y, Z) and b not in (AIR, STONE)]
check(len(flowing) >= 4, f"the water flows sideways ({len(flowing)} updates)")

n = len(c.received)
c.place_block((X, Y - 1, Z), 1, 0)   # the empty bucket takes the source back
c.pump(4.0)
u = updates(c, n)
check(((X, Y, Z), AIR) in u, "an empty bucket picks the source up")
dried = [p for p, b in u if b == AIR and p != (X, Y, Z)]
check(len(dried) >= 4, f"the flowing water dries up without its source ({len(dried)} blocks)")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
