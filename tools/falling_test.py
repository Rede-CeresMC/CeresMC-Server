"""Falling blocks (python tools/falling_test.py): sand with nothing below falls as an entity and
becomes a block again on the ground."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string

STONE = block_id("minecraft:stone")
SAND = block_id("minecraft:sand")
AIR = block_id("minecraft:air")
X, Z = 288, 266

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def last_block(c, pos, since):
    seen = [b for p, b in update_blocks(type("X", (), {"received": c.received[since:]})()) if p == pos]
    return seen[-1] if seen else None

c = Client("BotSand")
c.join()
c.move(X + 0.5, 71.62, Z - 3.5)
c.pump(0.5)
for y in range(70, 75):
    c.break_block((X, y, Z))
c.pump(0.5)
c.place_block((X, 69, Z), 1, STONE)   # replaces whatever is there
c.pump(0.3)
for y in (69, 70):
    c.place_block((X, y, Z), 1, STONE)
    c.pump(0.3)
c.place_block((X, 71, Z), 1, SAND)    # sand at y=72 on a stone column
c.pump(1.0)

n = len(c.received)
c.break_block((X, 71, Z))
c.pump(2.0)
check(any(p == 0x0d for p, _ in c.received[n:]), "the sand turns into a falling block entity (AddActor)")
check(last_block(c, (X, 72, Z), n) == AIR, "the sand left its old place")
check(last_block(c, (X, 71, Z), n) == SAND, f"the sand landed on the stone below ({last_block(c, (X, 71, Z), n)})")
check(any(p == 0x0e for p, _ in c.received[n:]), "the falling entity is removed after landing")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
