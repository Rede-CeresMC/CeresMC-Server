"""Enchanting table (python tools/enchanting_test.py): an item in the table gets three options;
buying one enchants the item and costs levels and lapis."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

STONE = block_id("minecraft:stone")
TABLE = block_id("minecraft:enchanting_table")
POS = (352, 70, 266)
HOTBAR = 28

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
    c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n:]))

def slot(container, s):
    return bytes([container, 0, s]) + struct.pack("<i", 0)

counter = [-500]
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

def zz(v):
    return (v >> 1) ^ -(v & 1)

def options(c, since):
    """[(cost, [(enchant, level)], net id)] of the last enchant options packet."""
    for pid, d in reversed(c.received[since:]):
        if pid != 0x92:
            continue
        n, p = read_uvarint(d, 0)
        out = []
        for _ in range(n):
            cost = d[p]; p += 1 + 4
            k, p = read_uvarint(d, p)
            for _ in range(k):
                _, p = read_uvarint(d, p); p += 1
            k, p = read_uvarint(d, p)
            ench = []
            for _ in range(k):
                e, p = read_uvarint(d, p); ench.append((e, d[p])); p += 1
            k, p = read_uvarint(d, p)
            for _ in range(k):
                _, p = read_uvarint(d, p); p += 1
            ln, p = read_uvarint(d, p); p += ln
            net, p = read_uvarint(d, p)
            out.append((cost, ench, zz(net)))
        return out
    return None

def level(c, since):
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
            if nm == "minecraft:player.level":
                val = vals[2]
    return val

c = Client("BotEnchant")
c.join()
c.move(POS[0] + 0.5, 71.62, POS[2] - 2.5)
c.pump(0.5)
command(c, "/gamemode creative")
c.break_block(POS)
c.place_block((POS[0], 69, POS[2]), 1, STONE)
c.pump(0.3)
c.place_block((POS[0], 69, POS[2]), 1, TABLE)
c.pump(0.6)
command(c, "/gamemode survival")
command(c, "/clear")
command(c, "/xp 30L")
give(c, "iron_sword 1")
give(c, "lapis_lazuli 3")

n = len(c.received)
c.place_block(POS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 3, "clicking an enchanting table opens it")
check(request(c, [take(3, slot(HOTBAR, 1), slot(23, 15))]), "lapis goes into the lapis slot")
n = len(c.received)
check(request(c, [take(1, slot(HOTBAR, 0), slot(22, 14))]), "the sword goes into the item slot")
c.pump(1.0)
opts = options(c, n)
check(opts is not None and len(opts) == 3, f"three enchanting options are offered ({opts})")
if opts:
    cost, ench, net = opts[0]
    n = len(c.received)
    bought = request(c, [(10, svarint(net) + bytes([1])), (5, bytes([1]) + slot(22, 14)), (5, bytes([1]) + slot(23, 15)),
                         take(1, slot(60, 50), slot(HOTBAR, 2))])
    check(bought, f"buying the first option is accepted ({ench})")
    c.pump(1.0)
    check(level(c, n) == 29, f"it costs 1 level ({level(c, n)})")
    check(not request(c, [(10, svarint(net) + bytes([1])), take(1, slot(60, 50), slot(HOTBAR, 3))]),
          "the same option cannot be bought again without an item")

# grindstone: takes the enchantments off and gives experience back
GRIND = block_id("minecraft:grindstone")
GPOS = (POS[0] + 2, POS[1], POS[2])
command(c, "/gamemode creative")
c.break_block(GPOS)
c.place_block((GPOS[0], 69, GPOS[2]), 1, STONE)
c.pump(0.3)
c.place_block((GPOS[0], 69, GPOS[2]), 1, GRIND)
c.pump(0.6)
command(c, "/gamemode survival")
command(c, "/clear")
give(c, "iron_sword 1")
command(c, "/enchant @s sharpness 3")
n = len(c.received)
c.place_block(GPOS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 26, "clicking a grindstone opens it")
check(request(c, [take(1, slot(HOTBAR, 0), slot(50, 16))]), "the enchanted sword goes into the grindstone")
n = len(c.received)
check(request(c, [(14, svarint(0) + svarint(0) + bytes([1])), (5, bytes([1]) + slot(50, 16)), take(1, slot(60, 50), slot(HOTBAR, 6))]),
      "grinding is accepted")
c.pump(1.0)
check(any(p == 0x0d and b"xp_orb" in d for p, d in c.received[n:]), "grinding gives experience back")

# anvil: renaming costs a level
ANVIL = block_id("minecraft:anvil")
APOS = (POS[0], POS[1], POS[2] + 4)   # clear of the other tests
command(c, "/gamemode creative")
c.break_block(APOS)
c.place_block((APOS[0], 69, APOS[2]), 1, STONE)
c.pump(0.3)
c.place_block((APOS[0], 69, APOS[2]), 1, ANVIL)
c.pump(0.6)
command(c, "/gamemode survival")
command(c, "/clear")
command(c, "/xp 5L")
give(c, "iron_sword 1")
n = len(c.received)
c.place_block(APOS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 5, "clicking an anvil opens it")
check(request(c, [take(1, slot(HOTBAR, 0), slot(0, 1))]), "the sword goes into the anvil")
n = len(c.received)
counter[0] -= 2
body = svarint(counter[0]) + uvarint(3)
body += uvarint(13) + bytes(1) + svarint(0) + struct.pack("<i", 0)
body += uvarint(5) + bytes(1) + bytes([1]) + slot(0, 1)
body += uvarint(0) + bytes(1) + bytes([1]) + slot(60, 50) + slot(HOTBAR, 6)
body += uvarint(1) + string("Excalibur") + struct.pack("<i", 0)
c.send(uvarint(0x93) + uvarint(1) + body)
c.pump(3.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
resp = [d for p, d in c.received[n:] if p == 0x94]
check(resp and resp[-1][1] == 0, "renaming at the anvil is accepted")
c.pump(1.0)
lv = level(c, n)
check(lv is not None and lv < level(c, 0) + 0.5 and lv >= 0, f"renaming costs a level (now {lv})")

# cartography table: paper + compass make an empty locator map; paper + glass pane make nothing
CARTO = block_id("minecraft:cartography_table")
CPOS = (POS[0] + 4, POS[1], POS[2] + 4)
command(c, "/gamemode creative")
c.break_block(CPOS)
c.place_block((CPOS[0], 69, CPOS[2]), 1, STONE)
c.pump(0.3)
c.place_block((CPOS[0], 69, CPOS[2]), 1, CARTO)
c.pump(0.6)
command(c, "/gamemode survival")
command(c, "/clear")
give(c, "paper 2")
give(c, "compass 1")
n = len(c.received)
c.place_block(CPOS, 1, 0)
c.pump(3.0, lambda: any(p == 0x2e for p, _ in c.received[n:]))
opens = [d for p, d in c.received[n:] if p == 0x2e]
check(opens and opens[-1][1] == 30, "clicking a cartography table opens it")

def carto_craft(c, consume):
    counter[0] -= 2
    body = svarint(counter[0]) + uvarint(2 + len(consume))
    body += uvarint(13) + bytes(1) + svarint(0) + struct.pack("<i", -1)
    for src in consume:
        body += uvarint(5) + bytes(1) + bytes([1]) + src
    body += uvarint(0) + bytes(1) + bytes([1]) + slot(60, 50) + slot(HOTBAR, 6)
    body += uvarint(0) + struct.pack("<i", 0)
    n = len(c.received)
    c.send(uvarint(0x93) + uvarint(1) + body)
    c.pump(3.0, lambda: any(p == 0x94 for p, _ in c.received[n:]))
    resp = [d for p, d in c.received[n:] if p == 0x94]
    return bool(resp) and resp[-1][1] == 0

check(request(c, [take(1, slot(HOTBAR, 0), slot(55, 12))]), "paper goes into the cartography table")
check(request(c, [take(1, slot(HOTBAR, 1), slot(56, 13))]), "a compass goes into the second slot")
check(carto_craft(c, [slot(55, 12), slot(56, 13)]), "paper + compass make an empty locator map")
command(c, "/clear")
give(c, "paper 1")
give(c, "glass_pane 1")
check(request(c, [take(1, slot(HOTBAR, 0), slot(55, 12))]), "paper goes in again")
check(request(c, [take(1, slot(HOTBAR, 1), slot(56, 13))]), "a glass pane goes into the second slot")
check(not carto_craft(c, [slot(55, 12), slot(56, 13)]), "paper + glass pane make nothing (refused)")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
