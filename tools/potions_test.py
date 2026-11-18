"""Potions (python tools/potions_test.py): drinking gives the potion's effect and an empty bottle,
a splash potion thrown at the ground hurts the thrower standing next to it."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, svarint, string, read_uvarint

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

def click_air(c):
    item = struct.pack("<hH", 0, 0) + uvarint(0) + bytes(1) + uvarint(0) + uvarint(0)
    data = svarint(1) + bytes([1]) + svarint(0) * 3 + bytes([255]) + svarint(0) + bytes(1) + item
    data += struct.pack("<fff", *c.pos) + struct.pack("<fff", 0, 0, 0) + uvarint(0) + bytes([1, 0])
    c.send(uvarint(0x1e) + svarint(0) + bytes(1) + uvarint(2) + uvarint(0) + data)

def zz(v):
    return (v >> 1) ^ -(v & 1)

def effects_added(c, since):
    out = []
    for pid, d in c.received[since:]:
        if pid != 0x1c:
            continue
        _, p = read_uvarint(d, 0)
        event = d[p]; p += 1
        eff, p = read_uvarint(d, p)
        if event == 1:
            out.append(zz(eff))
    return out

def lowest_health(c, since):
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
            if nm == "minecraft:health":
                val = vals[2] if val is None else min(val, vals[2])
    return val

X, Z = 330, 266
c = Client("BotPotion")
c.join()
c.move(X + 0.5, 71.62, Z + 0.5)
c.pump(0.5)
command(c, "/gamemode creative")
c.break_block((X, 70, Z)); c.break_block((X, 71, Z))
c.place_block((X, 69, Z), 1, block_id("minecraft:stone"))
c.pump(0.5)
command(c, "/gamemode survival")
command(c, "/clear")
give(c, "potion 1 31")   # Strength
n = len(c.received)
click_air(c)
c.pump(2.2)
check(5 in effects_added(c, n), f"drinking a Strength potion gives Strength ({effects_added(c, n)})")
check(any(p in (0x31, 0x32) for p, _ in c.received[n:]), "the bottle in the hand changes (empty bottle)")

command(c, "/clear")
give(c, "splash_potion 1 23")   # Harming
c.move(X + 0.5, 71.62, Z + 0.5, pitch=89.0)
c.pump(0.3)
n = len(c.received)
click_air(c)
c.pump(2.0)
hp = lowest_health(c, n)
check(hp is not None and hp < 20, f"a splash potion of Harming at your feet hurts ({hp})")
check(any(p == 0x19 for p, _ in c.received[n:]), "the potion bursts with particles")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
