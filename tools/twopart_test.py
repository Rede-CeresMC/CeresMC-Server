"""Two-part blocks (python tools/twopart_test.py): doors and beds take two blocks, a door opens as
a whole, and breaking one half removes the other."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string

STONE = block_id("minecraft:stone")
DOOR = block_id("minecraft:wooden_door")
BED = block_id("minecraft:bed")
AIR = block_id("minecraft:air")
DOOR_POS = (278, 70, 266)
BED_POS = (280, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def updates_since(c, n):
    out = {}
    for pos, b in update_blocks(type("X", (), {"received": c.received[n:]})()):
        out[pos] = b
    return out

def prepare(c, pos, extra=()):
    for p in (pos, (pos[0], pos[1] + 1, pos[2])) + tuple(extra):
        c.break_block(p)
    below = (pos[0], pos[1] - 1, pos[2])
    c.place_block(below, 1, STONE)
    for e in extra:
        c.place_block((e[0], e[1] - 1, e[2]), 1, STONE)
    c.pump(0.6)

c = Client("BotDoor")
c.join()
c.move(279.5, 71.62, 262.5)
c.pump(0.5)
command(c, "/gamemode creative")

prepare(c, DOOR_POS)
n = len(c.received)
c.place_block((DOOR_POS[0], DOOR_POS[1] - 1, DOOR_POS[2]), 1, DOOR)
c.pump(1.0)
u = updates_since(c, n)
top = (DOOR_POS[0], DOOR_POS[1] + 1, DOOR_POS[2])
check(u.get(DOOR_POS, AIR) != AIR and u.get(top, AIR) != AIR, "a door takes two blocks")
closed_low, closed_top = u.get(DOOR_POS), u.get(top)

n = len(c.received)
c.place_block(DOOR_POS, 1, 0)  # open it
c.pump(1.0)
u = updates_since(c, n)
check(DOOR_POS in u and top in u and u[DOOR_POS] != closed_low and u[top] != closed_top, "opening a door moves both halves")

n = len(c.received)
c.break_block(top)
c.pump(1.0)
u = updates_since(c, n)
check(u.get(top) == AIR and u.get(DOOR_POS) == AIR, "breaking the top half removes the bottom half")

head = (BED_POS[0], BED_POS[1], BED_POS[2] + 1)  # the bot looks south (yaw 0)
prepare(c, BED_POS, extra=(head,))
n = len(c.received)
c.place_block((BED_POS[0], BED_POS[1] - 1, BED_POS[2]), 1, BED)
c.pump(1.0)
u = updates_since(c, n)
check(u.get(BED_POS, AIR) != AIR and u.get(head, AIR) != AIR, "a bed takes two blocks (foot and head)")
check(any(p == 0x38 for p, _ in c.received[n:]), "the bed gets its colour block entity")
n = len(c.received)
c.break_block(head)
c.pump(1.0)
u = updates_since(c, n)
check(u.get(head) == AIR and u.get(BED_POS) == AIR, "breaking the head removes the foot")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
