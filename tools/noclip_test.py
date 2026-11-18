"""NoClip (python tools/noclip_test.py): blocks cannot be placed where a player stands, unless
that player has no-clip (walks through blocks without being a spectator)."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string

STONE = block_id("minecraft:stone")
X, Z = 334, 266
FEET = (X, 70, Z)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def placed(c, since):
    seen = [b for p, b in update_blocks(type("X", (), {"received": c.received[since:]})()) if p == FEET]
    return seen[-1] if seen else None

c = Client("BotNoclip")
c.join()
c.move(X + 0.5, 71.62, Z + 0.5)
c.pump(0.5)
command(c, "/gamemode creative")
c.break_block(FEET); c.break_block((X, 71, Z))
c.place_block((X, 69, Z), 1, STONE)
c.pump(0.6)
c.move(X + 0.5, 71.62, Z + 0.5)
c.pump(0.3)

n = len(c.received)
c.place_block((X, 69, Z), 1, STONE)
c.pump(1.0)
check(placed(c, n) != STONE, "a block cannot be placed where the player stands")

n = len(c.received)
command(c, "/noclip on")
check(any(p == 0xbb for p, _ in c.received[n:]), "/noclip sends the new abilities")
n = len(c.received)
c.place_block((X, 69, Z), 1, STONE)
c.pump(1.0)
check(placed(c, n) == STONE, "with no-clip, a block goes right where the player stands")
command(c, "/noclip off")
c.break_block(FEET)

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
