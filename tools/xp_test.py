"""Experience (python tools/xp_test.py): /xp in points and levels, ores drop orbs that the
player collects."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, string, read_uvarint, item_id, dropped_items
from bedrock_client import _auth_input

STONE = block_id("minecraft:stone")
ORE = block_id("minecraft:diamond_ore")
POS = (300, 70, 266)

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def attribute(c, name, since=0):
    val = None
    for pid, d in c.received[since:]:
        if pid != 0x1d:
            continue
        _, p = read_uvarint(d, 0)
        n, p = read_uvarint(d, p)
        for _ in range(n):
            vals = struct.unpack("<6f", d[p:p + 24]); p += 24
            ln, p = read_uvarint(d, p)
            nm = d[p:p + ln].decode(); p += ln
            _, p = read_uvarint(d, p)
            if nm == name:
                val = vals[2]
    return val

c = Client("BotXp")
c.join()
c.move(POS[0] + 0.5, 71.62, POS[2] - 2.5)
c.pump(0.5)
command(c, "/gamemode creative")
below = (POS[0], POS[1] - 1, POS[2])
c.break_block(POS)
c.place_block(below, 1, STONE)
c.pump(0.3)
c.place_block(below, 1, ORE)
c.pump(0.6)

n = len(c.received)
command(c, "/xp 10")
lvl, prog = attribute(c, "minecraft:player.level", n), attribute(c, "minecraft:player.experience", n)
check(lvl == 1 and prog is not None and abs(prog - 3 / 9) < 0.01, f"/xp 10 gives level 1 and 3/9 of the next ({lvl}, {prog})")
n = len(c.received)
command(c, "/xp 5L")
check(attribute(c, "minecraft:player.level", n) == 6, "/xp 5L adds five levels")

command(c, "/gamemode survival")
command(c, "/clear")
n0 = len(c.received)
command(c, "/give @s iron_pickaxe 1")
c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n0:]))
n = len(c.received)
c.send(_auth_input(c, [(0, POS, 1)]))
c.pump(0.9)
c.send(_auth_input(c, [(26, POS, 1)]))
c.pump(4.0, lambda: (attribute(c, "minecraft:player.level", n) or 6) > 6 or (attribute(c, "minecraft:player.experience", n) or 0) > 0.1)
orbs = [d for p, d in c.received[n:] if p == 0x0d and b"xp_orb" in d]
check(bool(orbs), f"mining diamond ore drops experience orbs ({len(orbs)})")
check(item_id("minecraft:diamond") in dropped_items(c, n), "diamond ore drops a diamond, not the ore block")
prog = attribute(c, "minecraft:player.experience", n)
check(prog is not None and prog > 0, f"the orbs fly to the player and are collected (progress {prog})")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
