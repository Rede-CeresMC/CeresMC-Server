"""Redstone (python tools/redstone_test.py): a lever on a lamp lights it and turns it off again,
a redstone block powers dust that lights a lamp at the end of the line, a button lights a lamp
for a moment, and a redstone block next to TNT lights it."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, update_blocks, block_id, uvarint, svarint, string

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
    command(c, "/clear")
    command(c, "/give @s " + what)
    c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def last(c, pos):
    v = None
    for q, b in update_blocks(c):
        if q == pos:
            v = b
    return v

BX, BZ = 380, 300
G = 69  # first layer above the flat ground
c = Client("BotRed")
c.join()
c.move(BX + 5.5, G + 1.62, BZ - 2.5)
c.pump(1.0)
command(c, "/gamemode creative")
STONE = block_id("minecraft:stone")
# a stone floor under the row (whatever the ground there is), the row itself cleared
command(c, "/give @s stone 64")
for x in range(BX - 1, BX + 14):
    c.place_block((x, G - 1, BZ), 1, STONE)
    c.pump(0.05)
c.pump(0.5)
for x in range(BX - 1, BX + 14):
    for y in (G, G + 1):
        c.break_block((x, y, BZ))
c.pump(0.5)

def put(item, pos, clicked=None, face=1, block=None):
    give(c, item)
    c.place_block(clicked or pos, face, block_id(block or "minecraft:" + item))
    c.pump(0.5)

def use(pos):
    c.place_block(pos, 1, 0)
    c.pump(0.5)

LAMP, LIT, AIR = block_id("minecraft:redstone_lamp"), block_id("minecraft:lit_redstone_lamp"), block_id("minecraft:air")

# 1. a lever on top of a lamp
L1 = (BX, G, BZ)
put("redstone_lamp", L1)
check(last(c, L1) == LAMP, "a redstone lamp is placed")
LEVER = (BX, G + 1, BZ)
put("lever", LEVER, clicked=L1)
check(last(c, LEVER) not in (None, AIR), "a lever is put on the lamp")
command(c, "/clear")
use(LEVER)
check(last(c, L1) == LIT, "switching the lever on lights the lamp")
use(LEVER)
c.pump(1.0)
check(last(c, L1) == LAMP, "switching it off turns the lamp off")

# 2. redstone block -> two dust -> lamp
R, D1, D2, L2 = (BX + 3, G, BZ), (BX + 4, G, BZ), (BX + 5, G, BZ), (BX + 6, G, BZ)
put("redstone_lamp", L2)
put("redstone", D1, block="minecraft:redstone_wire")
# in survival the held redstone item (which has no block of its own) places dust
command(c, "/gamemode survival")
give(c, "redstone 1")
c.place_block(D2, 1, 0)
c.pump(0.5)
command(c, "/gamemode creative")
check(last(c, D2) not in (None, AIR), "redstone dust is placed")
put("redstone_block", R)
c.pump(0.5)
plain = block_id("minecraft:redstone_wire")
check(last(c, D1) not in (None, AIR, plain), "the dust next to the redstone block carries power")
check(last(c, L2) == LIT, "the dust line lights the lamp at its end")
c.break_block(R)
c.pump(1.0)
check(last(c, L2) == LAMP, "without the redstone block the lamp goes out")

# 3. a button on top of a lamp
L3, BUTTON = (BX + 8, G, BZ), (BX + 8, G + 1, BZ)
put("redstone_lamp", L3)
put("stone_button", BUTTON, clicked=L3)
command(c, "/clear")
use(BUTTON)
check(last(c, L3) == LIT, "pressing a button lights the lamp")
c.pump(1.8)
check(last(c, L3) == LAMP, "the button springs back and the lamp goes out")

# 4. a repeater placed looking east carries power from the west to a lamp on its east
RB, REP, L4 = (BX + 1, G, BZ + 3), (BX + 2, G, BZ + 3), (BX + 3, G, BZ + 3)
command(c, "/give @s stone 8")
for x in (RB[0], REP[0], L4[0]):
    c.place_block((x, G - 1, BZ + 3), 1, STONE)
    c.pump(0.05)
for x in (RB[0], REP[0], L4[0]):
    c.break_block((x, G, BZ + 3))
c.pump(0.4)
put("redstone_lamp", L4)
c.move(BX - 1.5, G + 1.62, BZ + 3.5, 0.0, -90.0)  # looking east
c.pump(0.3)
put("repeater", REP, block="minecraft:unpowered_repeater")
check(last(c, REP) not in (None, AIR), "a repeater is placed")
put("redstone_block", RB)
c.pump(0.8)
check(last(c, L4) == LIT, "a repeater carries the power on to a lamp")
check(last(c, REP) not in (None, AIR, block_id("minecraft:unpowered_repeater")), "the repeater is powered")
n = len(c.received)
command(c, "/clear")
use(REP)
check(any(q == REP for q, _ in update_blocks(c)[-5:]), "using a repeater changes its delay")
c.move(BX + 5.5, G + 1.62, BZ - 2.5)
c.pump(0.3)

# 5. an observer placed looking east watches the west; a block there pulses the lamp east of it
W, OBS, L5 = (BX + 6, G, BZ + 3), (BX + 7, G, BZ + 3), (BX + 8, G, BZ + 3)
command(c, "/give @s stone 8")
for x in (W[0], OBS[0], L5[0]):
    c.place_block((x, G - 1, BZ + 3), 1, STONE)
    c.pump(0.05)
for x in (W[0], OBS[0], L5[0]):
    c.break_block((x, G, BZ + 3))
c.pump(0.4)
put("redstone_lamp", L5)
c.move(BX + 4.5, G + 1.62, BZ + 3.5, 0.0, -90.0)  # looking east
c.pump(0.3)
put("observer", OBS)
check(last(c, OBS) not in (None, AIR), "an observer is placed")
n = len(c.received)
put("stone", W)
c.pump(0.6)
lamp = [b for q, b in update_blocks(c)[-40:] if q == L5]
check(LIT in lamp, "a block appearing in front of the observer pulses the lamp behind it")
c.move(BX + 5.5, G + 1.62, BZ - 2.5)
c.pump(0.3)

# 6. a note block plays when power reaches it
NB, R3 = (BX + 10, G, BZ + 3), (BX + 11, G, BZ + 3)
command(c, "/give @s stone 4")
for x in (NB[0], R3[0]):
    c.place_block((x, G - 1, BZ + 3), 1, STONE)
    c.pump(0.05)
for x in (NB[0], R3[0]):
    c.break_block((x, G, BZ + 3))
c.pump(0.4)
put("noteblock", NB)
n = len(c.received)
put("redstone_block", R3)
c.pump(0.4)
check(any(p == 0x1a for p, _ in c.received[n:]), "a note block plays when powered (BlockEvent)")

# 7. a comparator reads a chest: empty gives nothing, a full stack lights the lamp in front
CH, CMP, L7 = (BX + 1, G, BZ + 6), (BX + 2, G, BZ + 6), (BX + 3, G, BZ + 6)
command(c, "/give @s stone 8")
for x in (CH[0], CMP[0], L7[0]):
    c.place_block((x, G - 1, BZ + 6), 1, STONE)
    c.pump(0.05)
for x in (CH[0], CMP[0], L7[0]):
    c.break_block((x, G, BZ + 6))
c.pump(0.4)
put("redstone_lamp", L7)
put("chest", CH)
c.move(BX - 1.5, G + 1.62, BZ + 6.5, 0.0, -90.0)  # looking east
c.pump(0.3)
put("comparator", CMP, block="minecraft:unpowered_comparator")
check(last(c, CMP) not in (None, AIR), "a comparator is placed")
c.pump(0.6)
check(last(c, L7) == LAMP, "an empty chest gives the comparator nothing")
command(c, "/gamemode survival")
give(c, "cobblestone 64")
n = len(c.received)
c.place_block(CH, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
# move the 64 cobblestone from hotbar slot 0 into the chest's first slot
req = uvarint(0x93) + uvarint(1) + svarint(-900) + uvarint(1)
req += uvarint(0) + bytes(1) + bytes([64]) + bytes([28, 0, 0]) + struct.pack("<i", 0) + bytes([7, 0, 0]) + struct.pack("<i", 0)
req += uvarint(0) + struct.pack("<i", 0)
c.send(req)
c.pump(0.6)
opens = [d for p, d in c.received[n:] if p == 0x2e]
if opens:
    c.send(uvarint(0x2f) + bytes([opens[-1][0], 0, 0]))
c.pump(1.0)
check(last(c, L7) == LIT, "a chest with items makes the comparator light the lamp")
command(c, "/gamemode creative")
c.move(BX + 5.5, G + 1.62, BZ - 2.5)
c.pump(0.3)

# 8. TNT next to a redstone block is lit
T, R2 = (BX + 11, G, BZ), (BX + 12, G, BZ)
put("tnt", T)
n = len(c.received)
put("redstone_block", R2)
c.pump(0.5)
check(last(c, T) == AIR, "a redstone block next to TNT lights it (the block goes)")
check(any(p == 0x0d and b"minecraft:tnt" in d for p, d in c.received[n:]), "primed TNT appears")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
