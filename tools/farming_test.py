"""Tools and farming (python tools/farming_test.py)."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string, item_id, dropped_items
from bedrock_client import _auth_input

STONE = block_id("minecraft:stone")
DIRT = block_id("minecraft:dirt")
LOG = block_id("minecraft:oak_log")
FARMLAND = block_id("minecraft:farmland")
FIRE = block_id("minecraft:fire")
X, Y, Z = 308, 70, 266
SOIL = (X, Y - 1, Z)
LOG_POS = (X + 2, Y, Z)
STONE_POS = (X + 4, Y - 1, Z)

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

def blocks_since(c, n):
    out = {}
    for pos, b in update_blocks(type("X", (), {"received": c.received[n:]})()):
        out[pos] = b
    return out

c = Client("BotFarm")
c.join()
c.move(X + 2.5, Y + 1.62, Z - 2.5)
c.pump(0.5)
command(c, "/gamemode creative")
for p in [(X, Y, Z), LOG_POS, (X + 4, Y, Z)]:
    c.break_block(p)
c.place_block(SOIL, 1, DIRT)          # replaces whatever is there with dirt
c.place_block(STONE_POS, 1, STONE)
c.place_block((LOG_POS[0], LOG_POS[1] - 1, LOG_POS[2]), 1, STONE)
c.pump(0.4)
c.place_block((LOG_POS[0], LOG_POS[1] - 1, LOG_POS[2]), 1, LOG)
c.pump(0.8)
command(c, "/gamemode survival")

hold(c, "wooden_hoe 1")
n = len(c.received)
c.place_block(SOIL, 1, 0)
c.pump(1.0)
check(blocks_since(c, n).get(SOIL) == FARMLAND, "a hoe turns dirt into farmland")

hold(c, "wheat_seeds 4")
n = len(c.received)
c.place_block(SOIL, 1, 0)
c.pump(1.0)
crop = blocks_since(c, n).get((X, Y, Z))
check(crop is not None and crop != block_id("minecraft:air"), "seeds plant wheat on farmland")

hold(c, "bone_meal 10")
n = len(c.received)
for _ in range(5):
    c.place_block((X, Y, Z), 1, 0)
    c.pump(0.3)
ripe = blocks_since(c, n).get((X, Y, Z))
check(ripe is not None and ripe != crop, "bone meal makes the wheat grow")

command(c, "/clear")
n = len(c.received)
c.send(_auth_input(c, [(0, (X, Y, Z), 1)]))
c.pump(0.2)
c.send(_auth_input(c, [(26, (X, Y, Z), 1)]))
c.pump(1.5)
check(item_id("minecraft:wheat") in dropped_items(c, n), f"ripe wheat drops wheat ({dropped_items(c, n)})")

hold(c, "wooden_axe 1")
n = len(c.received)
c.place_block(LOG_POS, 2, 0)
c.pump(1.0)
stripped = blocks_since(c, n).get(LOG_POS)
check(stripped is not None and stripped != LOG, "an axe strips the log")

hold(c, "flint_and_steel 1")
n = len(c.received)
c.place_block(STONE_POS, 1, 0)
c.pump(1.0)
check(blocks_since(c, n).get((X + 4, Y, Z)) == FIRE, "flint and steel lights a fire")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
