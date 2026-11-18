"""Survival mining speed (python tools/mining_test.py): breaking faster than the block allows is
refused, breaking after the right time works, and the tool decides whether the block drops."""
import sys, os, struct, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, update_blocks, uvarint, string, item_id, dropped_items
from bedrock_client import _auth_input

STONE = block_id("minecraft:stone")
DIRT = block_id("minecraft:dirt")
START, CREATIVE_DESTROY, PREDICT = 0, 13, 26
ROW = [(260 + i, 70, 270) for i in range(4)]

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + b"\x00" + string(""))
    c.pump(0.6)

def action(c, kind, pos):
    c.send(_auth_input(c, [(kind, pos, 1)]))

def state_after(c, pos, fn):
    """Block at pos the server reports after fn()."""
    n = len(c.received)
    fn()
    c.pump(1.0)
    seen = [b for p, b in update_blocks(c)[-50:] if p == pos]
    new = [d for p, d in c.received[n:] if p == 0x15]
    return seen[-1] if seen and new else None

def mine(c, pos, wait):
    action(c, START, pos)
    c.pump(wait)
    action(c, PREDICT, pos)

c = Client("BotMiner")
c.join()
c.move(261.5, 71.62, 267.5)
c.pump(0.5)
command(c, "/gamemode creative")
for i, pos in enumerate(ROW):
    below = (pos[0], pos[1] - 1, pos[2])
    c.break_block(pos)
    c.place_block(below, 1, STONE)
    c.pump(0.3)
    c.place_block(below, 1, STONE if i % 2 == 0 else DIRT)
    c.pump(0.3)
c.pump(1.0)

command(c, "/gamemode survival")
command(c, "/clear")
check(state_after(c, ROW[0], lambda: mine(c, ROW[0], 0)) == STONE, "stone broken instantly by hand is refused")
check(state_after(c, ROW[0], lambda: action(c, CREATIVE_DESTROY, ROW[0])) == STONE, "the creative destroy action is refused in survival")
check(state_after(c, ROW[1], lambda: mine(c, ROW[1], 0.2)) == DIRT, "dirt broken in 0.2 s is refused")
n = len(c.received)
check(state_after(c, ROW[1], lambda: mine(c, ROW[1], 0.8)) not in (DIRT, None), "dirt broken by hand after 0.8 s works")
events = []
for pid, d in c.received[n:]:
    if pid == 0x19:
        v, p = 0, 0
        shift = 0
        while True:
            byte = d[p]; p += 1
            v |= (byte & 0x7f) << shift; shift += 7
            if not byte & 0x80:
                break
        events.append((v >> 1) ^ -(v & 1))
check(any(3603 <= e <= 3608 for e in events), "the miner sees hit particles while mining")
check(2001 in events, "the miner sees the break particles")
check(any(p == 0x0f for p, _ in c.received[n:]), "the dirt dropped as an item")

command(c, "/give @s wooden_pickaxe 1")
check(state_after(c, ROW[2], lambda: mine(c, ROW[2], 0.3)) == STONE, "stone with a wooden pickaxe in 0.3 s is refused")
n = len(c.received)
check(state_after(c, ROW[2], lambda: mine(c, ROW[2], 1.2)) not in (STONE, None), "stone with a wooden pickaxe after 1.2 s works")
check(item_id("minecraft:cobblestone") in dropped_items(c, n), f"stone drops cobblestone ({dropped_items(c, n)})")

def slot_damage(c, since):
    """Damage NBT of the last InventorySlot the server sent for hotbar slot 0."""
    for pid, d in reversed(c.received[since:]):
        if pid == 0x32 and d[:2] == bytes(2):
            i = d.find(bytes([3, 6, 0]) + b"Damage")
            return struct.unpack("<i", d[i + 9:i + 13])[0] if i >= 0 else 0
    return None
check(slot_damage(c, n) == 1, f"the pickaxe lost 1 durability (Damage={slot_damage(c, n)})")

n = len(c.received)
command(c, "/enchant @s efficiency 5")
c.pump(1.0)
check(any(p == 0x32 and b"ench" in d for p, d in c.received[n:]), "/enchant puts the enchantment on the held item")
check(state_after(c, ROW[0], lambda: mine(c, ROW[0], 0.15)) not in (STONE, None), "Efficiency V mines stone almost instantly")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
